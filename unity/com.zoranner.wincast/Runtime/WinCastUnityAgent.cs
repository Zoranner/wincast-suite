using System;
using System.Runtime.InteropServices;
using UnityEngine;
using UnityEngine.EventSystems;
using Zoranner.WinCast.Capture;
using Zoranner.WinCast.Input;
using Zoranner.WinCast.Native;

namespace Zoranner.WinCast
{
    public sealed class WinCastUnityAgent : MonoBehaviour
    {
        [SerializeField]
        private int port = 7856;

        [SerializeField]
        private int captureWidth = 1280;

        [SerializeField]
        private int captureHeight = 720;

        [SerializeField]
        private int targetFps = 30;

        [SerializeField]
        private int bitrateKbps = 4000;

        [SerializeField]
        private int maxInputEventsPerFrame = 128;

        [SerializeField]
        private EventSystem eventSystem;

        [SerializeField]
        private MonoBehaviour remoteInputAdapterBehaviour;

        private readonly FinalFrameCapture finalFrameCapture = new();
        private readonly WinCastNativeBridge nativeBridge = new();
        private RemoteInputGateway remoteInputGateway;
        private bool started;

        public WinCastStatus Status => nativeBridge.IsCreated ? nativeBridge.GetStatus() : default;
        public Vector2Int VideoSize => new(Math.Max(1, captureWidth), Math.Max(1, captureHeight));
        public int MaxInputEventsPerFrame => Math.Max(1, maxInputEventsPerFrame);

        private void Awake()
        {
            if (eventSystem == null)
            {
                eventSystem = EventSystem.current;
            }
        }

        private void OnEnable()
        {
            StartAgent();
        }

        private void Update()
        {
            if (!started)
            {
                return;
            }

            SubmitCompletedFrames();
            remoteInputGateway.PollAndDispatch(VideoSize);
        }

        private void OnDisable()
        {
            StopAgent();
        }

        private void OnDestroy()
        {
            StopAgent();
            finalFrameCapture.Dispose();
            nativeBridge.Dispose();
        }

        public void StartAgent()
        {
            if (started)
            {
                return;
            }

            ApplyCommandLineOverrides();

            var config = new WinCastConfig
            {
                Port = Math.Max(1, port),
                Width = VideoSize.x,
                Height = VideoSize.y,
                Fps = Math.Max(1, targetFps),
                BitrateKbps = Math.Max(1, bitrateKbps),
            };

            try
            {
                nativeBridge.Create(config);
                nativeBridge.Start();

                remoteInputGateway = new RemoteInputGateway(
                    nativeBridge,
                    eventSystem,
                    ResolveRemoteInputAdapter(),
                    MaxInputEventsPerFrame
                );

                finalFrameCapture.Start(this, config.Width, config.Height, config.Fps);
                started = true;
            }
            catch
            {
                finalFrameCapture.Stop();
                TryShutdownNativeBridge();
                remoteInputGateway = null;
                throw;
            }
        }

        public void StopAgent()
        {
            if (!started)
            {
                return;
            }

            started = false;
            finalFrameCapture.Stop();
            nativeBridge.Shutdown();
        }

        private void SubmitCompletedFrames()
        {
            while (finalFrameCapture.TryDequeueFrame(out var frame))
            {
                using (frame)
                {
                    SubmitFrame(frame);
                }
            }
        }

        private void SubmitFrame(FrameReadbackResult frame)
        {
            var bytes = frame.Data.ToArray();
            var handle = GCHandle.Alloc(bytes, GCHandleType.Pinned);
            try
            {
                var nativeFrame = new WinCastFrame
                {
                    Data = handle.AddrOfPinnedObject(),
                    Width = frame.Width,
                    Height = frame.Height,
                    Stride = frame.Stride,
                    Format = WinCastFrameFormat.Rgba32,
                    TimestampMicroseconds = frame.TimestampMicroseconds,
                };

                nativeBridge.SubmitFrame(nativeFrame);
            }
            finally
            {
                handle.Free();
            }
        }

        private RemoteInputAdapter ResolveRemoteInputAdapter()
        {
            if (remoteInputAdapterBehaviour is RemoteInputAdapter adapter)
            {
                return adapter;
            }

            return null;
        }

        private void TryShutdownNativeBridge()
        {
            try
            {
                nativeBridge.Shutdown();
            }
            catch (Exception exception)
            {
                Debug.LogWarning($"Failed to clean up WinCast native runtime: {exception}");
            }
        }

        private void ApplyCommandLineOverrides()
        {
            var args = Environment.GetCommandLineArgs();
            for (var index = 0; index < args.Length; index += 1)
            {
                var arg = args[index];
                var name = arg;
                if (!IsPortArgument(name))
                {
                    continue;
                }

                var value = ReadCommandLineValue(args, ref index, ref name);
                port = ParsePositiveInt(name, value);
            }
        }

        private static string ReadCommandLineValue(string[] args, ref int index, ref string name)
        {
            var arg = args[index];
            var equalsIndex = arg.IndexOf('=');
            if (equalsIndex >= 0)
            {
                name = arg.Substring(0, equalsIndex);
                return arg.Substring(equalsIndex + 1);
            }

            if (index + 1 >= args.Length)
            {
                throw new ArgumentException($"Missing value for command line argument {arg}.");
            }

            var value = args[index + 1];
            if (value.StartsWith("--", StringComparison.Ordinal))
            {
                throw new ArgumentException($"Missing value for command line argument {arg}.");
            }

            index += 1;
            return value;
        }

        private static bool IsPortArgument(string arg)
        {
            return arg == "--wincast-port"
                || arg.StartsWith("--wincast-port=", StringComparison.Ordinal);
        }

        private static int ParsePositiveInt(string name, string value)
        {
            if (
                int.TryParse(
                    value,
                    System.Globalization.NumberStyles.Integer,
                    System.Globalization.CultureInfo.InvariantCulture,
                    out var result
                )
                && result > 0
            )
            {
                return result;
            }

            throw new ArgumentException(
                $"Command line argument {name} must be a positive integer."
            );
        }
    }
}
