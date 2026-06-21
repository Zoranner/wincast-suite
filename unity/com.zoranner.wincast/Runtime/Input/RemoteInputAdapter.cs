using UnityEngine;
using Zoranner.WinCast.Native;

namespace Zoranner.WinCast.Input
{
    public class RemoteInputAdapter : MonoBehaviour
    {
        public virtual void OnPointerMove(WinCastInputEvent inputEvent) { }

        public virtual void OnPointerDown(WinCastInputEvent inputEvent) { }

        public virtual void OnPointerUp(WinCastInputEvent inputEvent) { }

        public virtual void OnPointerScroll(WinCastInputEvent inputEvent) { }

        public virtual void OnKey(WinCastInputEvent inputEvent) { }

        public virtual void OnText(WinCastInputEvent inputEvent) { }
    }
}
