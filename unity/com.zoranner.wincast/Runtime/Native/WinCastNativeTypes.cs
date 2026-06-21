using System;
using System.Runtime.InteropServices;

namespace Zoranner.WinCast.Native
{
    public enum WinCastFrameFormat : int
    {
        Rgba32 = 1,
        Bgra32 = 2,
    }

    public enum WinCastInputEventType : int
    {
        Unknown = 0,
        PointerMove = 1,
        PointerDown = 2,
        PointerUp = 3,
        PointerScroll = 4,
        KeyDown = 5,
        KeyUp = 6,
        Text = 7,
    }

    public enum WinCastPointerButton : int
    {
        None = 0,
        Left = 1,
        Right = 2,
        Middle = 3,
    }

    public enum WinCastRuntimeState : int
    {
        Unknown = 0,
        Created = 1,
        Running = 2,
        Stopping = 3,
        Stopped = 4,
        Faulted = 5,
    }

    public sealed class WinCastConfig
    {
        public string ListenHost;
        public int Port;
        public int Width;
        public int Height;
        public int Fps;
        public int BitrateKbps;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct WinCastFrame
    {
        public IntPtr Data;
        public int Width;
        public int Height;
        public int Stride;
        public WinCastFrameFormat Format;
        public ulong TimestampMicroseconds;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct WinCastInputEvent
    {
        public WinCastInputEventType Type;
        public int PointerId;
        public float X;
        public float Y;
        public float DeltaX;
        public float DeltaY;
        public WinCastPointerButton Button;
        public int KeyCode;
        public uint UnicodeScalar;
        public ulong TimestampMicroseconds;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct WinCastStatus
    {
        public WinCastRuntimeState State;
        public int ConnectedClientCount;
        public ulong SubmittedFrameCount;
        public ulong DroppedFrameCount;
        public ulong SentFrameCount;
        public ulong ReceivedInputCount;
    }
}
