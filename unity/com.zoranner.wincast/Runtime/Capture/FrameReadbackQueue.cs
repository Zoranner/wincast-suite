using System;
using System.Collections.Generic;
using Unity.Collections;
using UnityEngine;
using UnityEngine.Rendering;

namespace Zoranner.WinCast.Capture
{
    public sealed class FrameReadbackQueue : IDisposable
    {
        private readonly Queue<FrameReadbackResult> completedFrames = new();
        private bool requestPending;
        private bool disposed;
        private int generation;

        public bool RequestPending => requestPending;
        public int CompletedCount => completedFrames.Count;

        public bool TryRequest(RenderTexture source, ulong timestampMicroseconds)
        {
            ThrowIfDisposed();

            if (source == null || requestPending)
            {
                return false;
            }

            var requestGeneration = generation;
            var width = source.width;
            var height = source.height;
            var stride = width * 4;
            requestPending = true;
            AsyncGPUReadback.Request(
                source,
                0,
                TextureFormat.RGBA32,
                request =>
                {
                    requestPending = false;

                    if (disposed || request.hasError || requestGeneration != generation)
                    {
                        return;
                    }

                    var data = request.GetData<byte>();
                    var copy = new NativeArray<byte>(
                        data.Length,
                        Allocator.Persistent,
                        NativeArrayOptions.UninitializedMemory
                    );
                    copy.CopyFrom(data);

                    completedFrames.Enqueue(
                        new FrameReadbackResult(width, height, stride, timestampMicroseconds, copy)
                    );
                }
            );

            return true;
        }

        public bool TryDequeue(out FrameReadbackResult frame)
        {
            ThrowIfDisposed();

            if (completedFrames.Count == 0)
            {
                frame = default;
                return false;
            }

            frame = completedFrames.Dequeue();
            return true;
        }

        public void Clear()
        {
            generation += 1;
            requestPending = false;

            while (completedFrames.Count > 0)
            {
                completedFrames.Dequeue().Dispose();
            }
        }

        public void Dispose()
        {
            if (disposed)
            {
                return;
            }

            disposed = true;
            Clear();
        }

        private void ThrowIfDisposed()
        {
            if (disposed)
            {
                throw new ObjectDisposedException(nameof(FrameReadbackQueue));
            }
        }
    }

    public readonly struct FrameReadbackResult : IDisposable
    {
        public FrameReadbackResult(
            int width,
            int height,
            int stride,
            ulong timestampMicroseconds,
            NativeArray<byte> data
        )
        {
            Width = width;
            Height = height;
            Stride = stride;
            TimestampMicroseconds = timestampMicroseconds;
            Data = data;
        }

        public int Width { get; }
        public int Height { get; }
        public int Stride { get; }
        public ulong TimestampMicroseconds { get; }
        public NativeArray<byte> Data { get; }

        public void Dispose()
        {
            if (Data.IsCreated)
            {
                Data.Dispose();
            }
        }
    }
}
