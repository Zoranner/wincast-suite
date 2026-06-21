using System;
using System.Collections;
using UnityEngine;

namespace Zoranner.WinCast.Capture
{
    public sealed class FinalFrameCapture : IDisposable
    {
        private readonly FrameReadbackQueue readbackQueue = new();
        private RenderTexture captureTexture;
        private Coroutine captureCoroutine;
        private MonoBehaviour coroutineHost;
        private bool running;
        private float nextCaptureTime;

        public bool IsRunning => running;
        public int Width { get; private set; }
        public int Height { get; private set; }
        public int TargetFps { get; private set; }

        public void Start(MonoBehaviour host, int width, int height, int targetFps)
        {
            if (running)
            {
                return;
            }

            if (host == null)
            {
                throw new ArgumentNullException(nameof(host));
            }

            Width = Math.Max(1, width);
            Height = Math.Max(1, height);
            TargetFps = Math.Max(1, targetFps);
            coroutineHost = host;
            captureTexture = CreateCaptureTexture(Width, Height);
            running = true;
            nextCaptureTime = 0f;
            captureCoroutine = coroutineHost.StartCoroutine(CaptureLoop());
        }

        public void Stop()
        {
            if (!running)
            {
                return;
            }

            running = false;

            if (captureCoroutine != null && coroutineHost != null)
            {
                coroutineHost.StopCoroutine(captureCoroutine);
            }

            captureCoroutine = null;
            coroutineHost = null;
            readbackQueue.Clear();

            if (captureTexture != null)
            {
                captureTexture.Release();
                UnityEngine.Object.Destroy(captureTexture);
                captureTexture = null;
            }
        }

        public bool TryDequeueFrame(out FrameReadbackResult frame)
        {
            return readbackQueue.TryDequeue(out frame);
        }

        public void Dispose()
        {
            Stop();
            readbackQueue.Dispose();
        }

        private IEnumerator CaptureLoop()
        {
            var waitForEndOfFrame = new WaitForEndOfFrame();
            var captureInterval = 1f / TargetFps;

            while (running)
            {
                yield return waitForEndOfFrame;

                if (Time.unscaledTime < nextCaptureTime)
                {
                    continue;
                }

                nextCaptureTime = Time.unscaledTime + captureInterval;

                if (readbackQueue.RequestPending)
                {
                    continue;
                }

                ScreenCapture.CaptureScreenshotIntoRenderTexture(captureTexture);
                readbackQueue.TryRequest(captureTexture, CurrentTimestampMicroseconds());
            }
        }

        private static RenderTexture CreateCaptureTexture(int width, int height)
        {
            var texture = new RenderTexture(width, height, 0, RenderTextureFormat.ARGB32)
            {
                name = "WinCast Final Frame",
                useMipMap = false,
                autoGenerateMips = false,
            };

            texture.Create();
            return texture;
        }

        private static ulong CurrentTimestampMicroseconds()
        {
            return (ulong)(Time.realtimeSinceStartupAsDouble * 1_000_000d);
        }
    }
}
