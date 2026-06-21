using System;
using System.Runtime.InteropServices;
using System.Text;

namespace Zoranner.WinCast.Native
{
    public sealed class WinCastNativeBridge : IDisposable
    {
        private const string LibraryName = "wincast_unity_native";

        private ulong handle;
        private bool disposed;

        public bool IsCreated => handle != 0UL;

        public void Create(WinCastConfig config)
        {
            ThrowIfDisposed();

            if (IsCreated)
            {
                throw new InvalidOperationException(
                    "WinCast native runtime has already been created."
                );
            }

            var configJson = BuildConfigJson(config);
            var configBytes = ToNullTerminatedUtf8(configJson);
            var configHandle = GCHandle.Alloc(configBytes, GCHandleType.Pinned);

            try
            {
                handle = wincast_unity_create(configHandle.AddrOfPinnedObject());
                if (handle == 0UL)
                {
                    ThrowNativeFailure("create native runtime");
                }
            }
            finally
            {
                configHandle.Free();
            }
        }

        public void Start()
        {
            ThrowIfDisposed();
            ThrowIfNotCreated();
            ThrowIfFailed(wincast_unity_start(handle), "start native runtime");
        }

        public void SubmitFrame(WinCastFrame frame)
        {
            ThrowIfDisposed();
            ThrowIfNotCreated();
            ThrowIfFailed(
                wincast_unity_submit_frame(
                    handle,
                    frame.Data,
                    (uint)frame.Width,
                    (uint)frame.Height,
                    (uint)frame.Stride,
                    ToNativeFrameFormat(frame.Format),
                    frame.TimestampMicroseconds * 1000UL
                ),
                "submit frame"
            );
        }

        public int PollInput(WinCastInputEvent[] eventsBuffer)
        {
            return PollInputCore(eventsBuffer, throwOnRuntimeFailure: false);
        }

        public int PollInputOrThrow(WinCastInputEvent[] eventsBuffer)
        {
            return PollInputCore(eventsBuffer, throwOnRuntimeFailure: true);
        }

        private int PollInputCore(WinCastInputEvent[] eventsBuffer, bool throwOnRuntimeFailure)
        {
            ThrowIfDisposed();
            ThrowIfNotCreated();

            if (eventsBuffer == null)
            {
                throw new ArgumentNullException(nameof(eventsBuffer));
            }

            if (eventsBuffer.Length == 0)
            {
                return 0;
            }

            var bufferHandle = GCHandle.Alloc(eventsBuffer, GCHandleType.Pinned);
            try
            {
                var bufferLengthBytes = checked(
                    (UIntPtr)(eventsBuffer.Length * Marshal.SizeOf<WinCastInputEvent>())
                );
                var count = checked(
                    (int)wincast_unity_poll_input(
                        handle,
                        bufferHandle.AddrOfPinnedObject(),
                        bufferLengthBytes
                    )
                );
                if (count > 0)
                {
                    return Math.Min(count, eventsBuffer.Length);
                }

                if (throwOnRuntimeFailure)
                {
                    ThrowIfPollInputFailed();
                }

                return 0;
            }
            finally
            {
                bufferHandle.Free();
            }
        }

        public WinCastStatus GetStatus()
        {
            ThrowIfDisposed();
            ThrowIfNotCreated();

            return new WinCastStatus { State = ToManagedState(wincast_unity_get_status(handle)) };
        }

        public string GetLastError()
        {
            var buffer = new byte[1024];
            var written = checked(
                (int)wincast_unity_get_last_error(buffer, (UIntPtr)buffer.Length)
            );
            if (written <= 0)
            {
                return string.Empty;
            }

            var byteCount = Math.Min(written, buffer.Length);
            if (byteCount > 0 && buffer[byteCount - 1] == 0)
            {
                byteCount -= 1;
            }

            return Encoding.UTF8.GetString(buffer, 0, byteCount);
        }

        public void Shutdown()
        {
            if (!IsCreated)
            {
                return;
            }

            var currentHandle = handle;
            handle = 0UL;
            ThrowIfFailed(wincast_unity_shutdown(currentHandle), "shutdown native runtime");
        }

        public void Dispose()
        {
            if (disposed)
            {
                return;
            }

            disposed = true;
            Shutdown();
            GC.SuppressFinalize(this);
        }

        private static byte[] ToNullTerminatedUtf8(string value)
        {
            value ??= string.Empty;
            var bytes = Encoding.UTF8.GetBytes(value);
            var result = new byte[bytes.Length + 1];
            Buffer.BlockCopy(bytes, 0, result, 0, bytes.Length);
            return result;
        }

        private static string BuildConfigJson(WinCastConfig config)
        {
            return string.Format(
                System.Globalization.CultureInfo.InvariantCulture,
                "{{\"listen_addr\":\"127.0.0.1:{0}\",\"width\":{1},\"height\":{2},\"fps\":{3},\"bitrate_kbps\":{4}}}",
                config.Port,
                config.Width,
                config.Height,
                config.Fps,
                config.BitrateKbps
            );
        }

        private static WincastUnityFrameFormat ToNativeFrameFormat(WinCastFrameFormat format)
        {
            return format switch
            {
                WinCastFrameFormat.Rgba32 => WincastUnityFrameFormat.Rgba8,
                WinCastFrameFormat.Bgra32 => WincastUnityFrameFormat.Bgra8,
                _ => WincastUnityFrameFormat.Rgba8,
            };
        }

        private static WinCastRuntimeState ToManagedState(WincastUnityStatus status)
        {
            return status switch
            {
                WincastUnityStatus.Created => WinCastRuntimeState.Created,
                WincastUnityStatus.Started => WinCastRuntimeState.Running,
                WincastUnityStatus.Stopped => WinCastRuntimeState.Stopped,
                WincastUnityStatus.Failed => WinCastRuntimeState.Faulted,
                _ => WinCastRuntimeState.Unknown,
            };
        }

        private void ThrowIfDisposed()
        {
            if (disposed)
            {
                throw new ObjectDisposedException(nameof(WinCastNativeBridge));
            }
        }

        private void ThrowIfNotCreated()
        {
            if (!IsCreated)
            {
                throw new InvalidOperationException("WinCast native runtime has not been created.");
            }
        }

        private void ThrowIfFailed(int result, string operation)
        {
            if (result == 0)
            {
                return;
            }

            var error = GetLastError();
            if (string.IsNullOrWhiteSpace(error))
            {
                error = $"native error code {result}";
            }

            throw new InvalidOperationException($"Failed to {operation}: {error}");
        }

        private void ThrowNativeFailure(string operation)
        {
            var error = GetLastError();
            if (string.IsNullOrWhiteSpace(error))
            {
                error = "native call failed";
            }

            throw new InvalidOperationException($"Failed to {operation}: {error}");
        }

        private void ThrowIfPollInputFailed()
        {
            var error = GetLastError();
            if (string.IsNullOrWhiteSpace(error))
            {
                return;
            }

            throw new InvalidOperationException($"Failed to poll native input: {error}");
        }

        [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
        private static extern ulong wincast_unity_create(IntPtr configJson);

        [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
        private static extern int wincast_unity_start(ulong handle);

        [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
        private static extern int wincast_unity_submit_frame(
            ulong handle,
            IntPtr framePtr,
            uint width,
            uint height,
            uint strideBytes,
            WincastUnityFrameFormat format,
            ulong timestampNs
        );

        [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
        private static extern UIntPtr wincast_unity_poll_input(
            ulong handle,
            IntPtr outputBuffer,
            UIntPtr bufferLen
        );

        [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
        private static extern WincastUnityStatus wincast_unity_get_status(ulong handle);

        [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
        private static extern UIntPtr wincast_unity_get_last_error(
            [Out] byte[] buffer,
            UIntPtr bufferLength
        );

        [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
        private static extern int wincast_unity_shutdown(ulong handle);

        private enum WincastUnityStatus : int
        {
            Invalid = -1,
            Created = 0,
            Started = 1,
            Stopped = 2,
            Failed = 3,
        }

        private enum WincastUnityFrameFormat : int
        {
            Rgba8 = 0,
            Bgra8 = 1,
        }
    }
}
