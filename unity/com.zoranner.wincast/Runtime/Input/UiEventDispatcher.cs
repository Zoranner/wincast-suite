using System;
using System.Collections.Generic;
using System.Reflection;
using UnityEngine;
using UnityEngine.EventSystems;
using UnityEngine.UI;
using Zoranner.WinCast.Native;

namespace Zoranner.WinCast.Input
{
    public sealed class UiEventDispatcher
    {
        private const BindingFlags PublicInstance = BindingFlags.Public | BindingFlags.Instance;

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
                case WinCastInputEventType.Text:
                    return DispatchText(inputEvent);
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

        private bool DispatchText(WinCastInputEvent inputEvent)
        {
            var text = ToText(inputEvent.UnicodeScalar);
            if (string.IsNullOrEmpty(text))
            {
                return false;
            }

            var target = EventSystem.currentSelectedGameObject;
            if (target == null)
            {
                return false;
            }

            return TryDispatchUnityInputField(target, text)
                || TryDispatchTmpInputField(target, text);
        }

        private static bool TryDispatchUnityInputField(GameObject target, string text)
        {
            var inputField = target.GetComponentInParent<InputField>();
            if (inputField == null || !inputField.isActiveAndEnabled || !inputField.interactable)
            {
                return false;
            }

            var currentText = inputField.text ?? string.Empty;
            var selectionStart = Mathf.Clamp(
                Math.Min(inputField.selectionAnchorPosition, inputField.selectionFocusPosition),
                0,
                currentText.Length
            );
            var selectionEnd = Mathf.Clamp(
                Math.Max(inputField.selectionAnchorPosition, inputField.selectionFocusPosition),
                0,
                currentText.Length
            );
            var caretPosition = selectionStart + text.Length;

            inputField.text = currentText
                .Remove(selectionStart, selectionEnd - selectionStart)
                .Insert(selectionStart, text);
            inputField.caretPosition = caretPosition;
            inputField.selectionAnchorPosition = caretPosition;
            inputField.selectionFocusPosition = caretPosition;
            inputField.ForceLabelUpdate();
            return true;
        }

        private static bool TryDispatchTmpInputField(GameObject target, string text)
        {
            var inputField = FindTmpInputField(target);
            if (inputField == null)
            {
                return false;
            }

            var type = inputField.GetType();
            if (
                !GetBoolProperty(type, inputField, "isActiveAndEnabled", true)
                || !GetBoolProperty(type, inputField, "interactable", true)
            )
            {
                return false;
            }

            var currentText = GetStringProperty(type, inputField, "text");
            if (currentText == null)
            {
                return false;
            }

            var fallbackCaret = GetIntProperty(type, inputField, "caretPosition", 0);
            var selectionStart = Mathf.Clamp(
                Math.Min(
                    GetIntProperty(type, inputField, "selectionAnchorPosition", fallbackCaret),
                    GetIntProperty(type, inputField, "selectionFocusPosition", fallbackCaret)
                ),
                0,
                currentText.Length
            );
            var selectionEnd = Mathf.Clamp(
                Math.Max(
                    GetIntProperty(type, inputField, "selectionAnchorPosition", fallbackCaret),
                    GetIntProperty(type, inputField, "selectionFocusPosition", fallbackCaret)
                ),
                0,
                currentText.Length
            );
            var caretPosition = selectionStart + text.Length;
            var nextText = currentText
                .Remove(selectionStart, selectionEnd - selectionStart)
                .Insert(selectionStart, text);

            if (!SetStringProperty(type, inputField, "text", nextText))
            {
                return false;
            }

            SetIntProperty(type, inputField, "caretPosition", caretPosition);
            SetIntProperty(type, inputField, "selectionAnchorPosition", caretPosition);
            SetIntProperty(type, inputField, "selectionFocusPosition", caretPosition);
            InvokePublicMethod(type, inputField, "ForceLabelUpdate");
            return true;
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

        private static string ToText(uint unicodeScalar)
        {
            if (unicodeScalar == 0 || unicodeScalar > 0x10FFFF)
            {
                return string.Empty;
            }

            if (unicodeScalar >= 0xD800 && unicodeScalar <= 0xDFFF)
            {
                return string.Empty;
            }

            return char.ConvertFromUtf32((int)unicodeScalar);
        }

        private static Component FindTmpInputField(GameObject target)
        {
            var behaviours = target.GetComponentsInParent<MonoBehaviour>(true);
            for (var index = 0; index < behaviours.Length; index += 1)
            {
                var behaviour = behaviours[index];
                if (behaviour != null && behaviour.GetType().FullName == "TMPro.TMP_InputField")
                {
                    return behaviour;
                }
            }

            return null;
        }

        private static string GetStringProperty(Type type, object instance, string propertyName)
        {
            return type.GetProperty(propertyName, PublicInstance)?.GetValue(instance) as string;
        }

        private static bool GetBoolProperty(
            Type type,
            object instance,
            string propertyName,
            bool defaultValue
        )
        {
            var value = type.GetProperty(propertyName, PublicInstance)?.GetValue(instance);
            return value is bool result ? result : defaultValue;
        }

        private static int GetIntProperty(
            Type type,
            object instance,
            string propertyName,
            int defaultValue
        )
        {
            var value = type.GetProperty(propertyName, PublicInstance)?.GetValue(instance);
            return value is int result ? result : defaultValue;
        }

        private static bool SetStringProperty(
            Type type,
            object instance,
            string propertyName,
            string value
        )
        {
            var property = type.GetProperty(propertyName, PublicInstance);
            if (property == null || !property.CanWrite)
            {
                return false;
            }

            property.SetValue(instance, value);
            return true;
        }

        private static void SetIntProperty(
            Type type,
            object instance,
            string propertyName,
            int value
        )
        {
            var property = type.GetProperty(propertyName, PublicInstance);
            if (property != null && property.CanWrite)
            {
                property.SetValue(instance, value);
            }
        }

        private static void InvokePublicMethod(Type type, object instance, string methodName)
        {
            type.GetMethod(methodName, PublicInstance)?.Invoke(instance, Array.Empty<object>());
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
