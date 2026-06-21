using System;
using UnityEngine;
using UnityEngine.EventSystems;
using Zoranner.WinCast.Native;

namespace Zoranner.WinCast.Input
{
    public sealed class RemoteInputGateway
    {
        private readonly WinCastNativeBridge nativeBridge;
        private readonly WinCastInputEvent[] inputBuffer;
        private UiEventDispatcher uiEventDispatcher;
        private RemoteInputAdapter remoteInputAdapter;

        public RemoteInputGateway(
            WinCastNativeBridge nativeBridge,
            EventSystem eventSystem,
            RemoteInputAdapter remoteInputAdapter,
            int maxEventsPerFrame
        )
        {
            this.nativeBridge =
                nativeBridge ?? throw new ArgumentNullException(nameof(nativeBridge));
            this.remoteInputAdapter = remoteInputAdapter;
            inputBuffer = new WinCastInputEvent[Math.Max(1, maxEventsPerFrame)];
            uiEventDispatcher = new UiEventDispatcher(eventSystem);
        }

        public void SetEventSystem(EventSystem eventSystem)
        {
            uiEventDispatcher = new UiEventDispatcher(eventSystem);
        }

        public void SetRemoteInputAdapter(RemoteInputAdapter adapter)
        {
            remoteInputAdapter = adapter;
        }

        public void PollAndDispatch(Vector2Int videoSize)
        {
            if (!nativeBridge.IsCreated)
            {
                return;
            }

            var count = nativeBridge.PollInputOrThrow(inputBuffer);
            for (var index = 0; index < count; index += 1)
            {
                Dispatch(inputBuffer[index], videoSize);
            }
        }

        private void Dispatch(WinCastInputEvent inputEvent, Vector2Int videoSize)
        {
            var handledByUi = uiEventDispatcher.Dispatch(inputEvent, videoSize);
            if (!handledByUi)
            {
                DispatchToAdapter(inputEvent);
            }
        }

        private void DispatchToAdapter(WinCastInputEvent inputEvent)
        {
            if (remoteInputAdapter == null)
            {
                return;
            }

            switch (inputEvent.Type)
            {
                case WinCastInputEventType.PointerMove:
                    remoteInputAdapter.OnPointerMove(inputEvent);
                    break;
                case WinCastInputEventType.PointerDown:
                    remoteInputAdapter.OnPointerDown(inputEvent);
                    break;
                case WinCastInputEventType.PointerUp:
                    remoteInputAdapter.OnPointerUp(inputEvent);
                    break;
                case WinCastInputEventType.PointerScroll:
                    remoteInputAdapter.OnPointerScroll(inputEvent);
                    break;
                case WinCastInputEventType.KeyDown:
                case WinCastInputEventType.KeyUp:
                    remoteInputAdapter.OnKey(inputEvent);
                    break;
                case WinCastInputEventType.Text:
                    remoteInputAdapter.OnText(inputEvent);
                    break;
            }
        }
    }
}
