using System.Collections.Generic;
using UnityEngine;
using UnityEngine.EventSystems;
using Zoranner.WinCast.Native;

namespace Zoranner.WinCast.Input
{
    public sealed class UiEventDispatcher
    {
        private readonly List<RaycastResult> raycastResults = new();
        private GameObject currentPointerTarget;
        private GameObject currentPressTarget;
        private GameObject currentDragTarget;
        private bool dragging;
        private bool currentDragHandled;

        public UiEventDispatcher(EventSystem eventSystem)
        {
            EventSystem = eventSystem;
        }

        public EventSystem EventSystem { get; }

        public bool Dispatch(WinCastInputEvent inputEvent, Vector2Int videoSize)
        {
            if (EventSystem == null)
            {
                return false;
            }

            switch (inputEvent.Type)
            {
                case WinCastInputEventType.PointerMove:
                    return DispatchPointerMove(inputEvent, videoSize);
                case WinCastInputEventType.PointerDown:
                    return DispatchPointerDown(inputEvent, videoSize);
                case WinCastInputEventType.PointerUp:
                    return DispatchPointerUp(inputEvent, videoSize);
                case WinCastInputEventType.PointerScroll:
                    return DispatchPointerScroll(inputEvent, videoSize);
                default:
                    return false;
            }
        }

        private bool DispatchPointerMove(WinCastInputEvent inputEvent, Vector2Int videoSize)
        {
            var pointerEventData = CreatePointerEventData(inputEvent, videoSize);
            var target = RaycastCurrent(pointerEventData);
            var handled = false;

            if (target != currentPointerTarget)
            {
                if (currentPointerTarget != null)
                {
                    handled |= ExecuteEvents.Execute(
                        currentPointerTarget,
                        pointerEventData,
                        ExecuteEvents.pointerExitHandler
                    );
                }

                if (target != null)
                {
                    handled |= ExecuteEvents.Execute(
                        target,
                        pointerEventData,
                        ExecuteEvents.pointerEnterHandler
                    );
                }

                currentPointerTarget = target;
            }

            if (currentDragTarget != null)
            {
                if (!dragging)
                {
                    handled |= ExecuteEvents.Execute(
                        currentDragTarget,
                        pointerEventData,
                        ExecuteEvents.beginDragHandler
                    );
                    dragging = true;
                }

                handled |= ExecuteEvents.Execute(
                    currentDragTarget,
                    pointerEventData,
                    ExecuteEvents.dragHandler
                );
            }

            return handled || currentDragHandled;
        }

        private bool DispatchPointerDown(WinCastInputEvent inputEvent, Vector2Int videoSize)
        {
            var pointerEventData = CreatePointerEventData(inputEvent, videoSize);
            currentPointerTarget = RaycastCurrent(pointerEventData);

            if (currentPointerTarget == null)
            {
                currentPressTarget = null;
                currentDragTarget = null;
                dragging = false;
                currentDragHandled = false;
                return false;
            }

            pointerEventData.pointerPressRaycast = pointerEventData.pointerCurrentRaycast;
            pointerEventData.pressPosition = pointerEventData.position;
            currentPressTarget = ExecuteEvents.ExecuteHierarchy(
                currentPointerTarget,
                pointerEventData,
                ExecuteEvents.pointerDownHandler
            );
            var handled = currentPressTarget != null;
            currentPressTarget ??= currentPointerTarget;
            pointerEventData.pointerPress = currentPressTarget;
            pointerEventData.rawPointerPress = currentPointerTarget;

            currentDragTarget = ExecuteEvents.GetEventHandler<IDragHandler>(currentPointerTarget);
            dragging = false;
            currentDragHandled = currentDragTarget != null;

            if (currentDragTarget != null)
            {
                currentDragHandled |= ExecuteEvents.Execute(
                    currentDragTarget,
                    pointerEventData,
                    ExecuteEvents.initializePotentialDrag
                );
            }

            return handled || currentDragHandled;
        }

        private bool DispatchPointerUp(WinCastInputEvent inputEvent, Vector2Int videoSize)
        {
            var pointerEventData = CreatePointerEventData(inputEvent, videoSize);
            var releaseTarget = RaycastCurrent(pointerEventData);
            var pointerUpHandled = false;
            var clickHandled = false;
            var endDragHandled = false;
            var dropHandled = false;

            if (currentPressTarget != null)
            {
                pointerEventData.pointerPress = currentPressTarget;
                pointerEventData.rawPointerPress = currentPointerTarget;
                pointerUpHandled = ExecuteEvents.Execute(
                    currentPressTarget,
                    pointerEventData,
                    ExecuteEvents.pointerUpHandler
                );

                var clickTarget = ExecuteEvents.GetEventHandler<IPointerClickHandler>(
                    releaseTarget
                );
                if (clickTarget == currentPressTarget)
                {
                    clickHandled = ExecuteEvents.Execute(
                        currentPressTarget,
                        pointerEventData,
                        ExecuteEvents.pointerClickHandler
                    );
                }
            }

            if (currentDragTarget != null)
            {
                if (dragging)
                {
                    endDragHandled = ExecuteEvents.Execute(
                        currentDragTarget,
                        pointerEventData,
                        ExecuteEvents.endDragHandler
                    );
                }

                if (releaseTarget != null)
                {
                    dropHandled =
                        ExecuteEvents.ExecuteHierarchy(
                            releaseTarget,
                            pointerEventData,
                            ExecuteEvents.dropHandler
                        ) != null;
                }
            }

            var handled =
                pointerUpHandled
                || clickHandled
                || endDragHandled
                || dropHandled
                || currentDragHandled;
            currentPressTarget = null;
            currentDragTarget = null;
            dragging = false;
            currentDragHandled = false;
            currentPointerTarget = releaseTarget;
            return handled;
        }

        private bool DispatchPointerScroll(WinCastInputEvent inputEvent, Vector2Int videoSize)
        {
            var pointerEventData = CreatePointerEventData(inputEvent, videoSize);
            pointerEventData.scrollDelta = new Vector2(inputEvent.DeltaX, inputEvent.DeltaY);
            var target = RaycastCurrent(pointerEventData);

            if (target == null)
            {
                currentPointerTarget = null;
                return false;
            }

            var handled = ExecuteEvents.ExecuteHierarchy(
                target,
                pointerEventData,
                ExecuteEvents.scrollHandler
            );
            currentPointerTarget = target;
            return handled != null;
        }

        private PointerEventData CreatePointerEventData(
            WinCastInputEvent inputEvent,
            Vector2Int videoSize
        )
        {
            return new PointerEventData(EventSystem)
            {
                pointerId = inputEvent.PointerId,
                button = ToInputButton(inputEvent.Button),
                position = ToScreenPosition(inputEvent, videoSize),
                delta = ToScreenDelta(inputEvent, videoSize),
                scrollDelta = Vector2.zero,
            };
        }

        private GameObject RaycastCurrent(PointerEventData pointerEventData)
        {
            raycastResults.Clear();
            EventSystem.RaycastAll(pointerEventData, raycastResults);
            pointerEventData.pointerCurrentRaycast = FindFirstRaycast(raycastResults);
            return pointerEventData.pointerCurrentRaycast.gameObject;
        }

        private static RaycastResult FindFirstRaycast(List<RaycastResult> candidates)
        {
            for (var index = 0; index < candidates.Count; index += 1)
            {
                if (candidates[index].gameObject != null)
                {
                    return candidates[index];
                }
            }

            return default;
        }

        private static Vector2 ToScreenPosition(WinCastInputEvent inputEvent, Vector2Int videoSize)
        {
            var width = Mathf.Max(1, videoSize.x);
            var height = Mathf.Max(1, videoSize.y);
            var x = Mathf.Clamp01(inputEvent.X / width) * Screen.width;
            var y = (1f - Mathf.Clamp01(inputEvent.Y / height)) * Screen.height;
            return new Vector2(x, y);
        }

        private static Vector2 ToScreenDelta(WinCastInputEvent inputEvent, Vector2Int videoSize)
        {
            var width = Mathf.Max(1, videoSize.x);
            var height = Mathf.Max(1, videoSize.y);
            var x = inputEvent.DeltaX / width * Screen.width;
            var y = -inputEvent.DeltaY / height * Screen.height;
            return new Vector2(x, y);
        }

        private static PointerEventData.InputButton ToInputButton(WinCastPointerButton button)
        {
            return button switch
            {
                WinCastPointerButton.Right => PointerEventData.InputButton.Right,
                WinCastPointerButton.Middle => PointerEventData.InputButton.Middle,
                _ => PointerEventData.InputButton.Left,
            };
        }
    }
}
