use std::collections::VecDeque;

use crate::error::{UnityNativeError, UnityNativeResult};
use wincast_protocol::input::{ButtonState, InputEvent, MouseButton};

pub(crate) const INPUT_QUEUE_CAPACITY: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub enum WincastUnityInputEventType {
    Unknown = 0,
    PointerMove = 1,
    PointerDown = 2,
    PointerUp = 3,
    PointerScroll = 4,
    KeyDown = 5,
    KeyUp = 6,
    Text = 7,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub enum WincastUnityPointerButton {
    None = 0,
    Left = 1,
    Right = 2,
    Middle = 3,
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct WincastUnityInputEvent {
    pub event_type: WincastUnityInputEventType,
    pub pointer_id: i32,
    pub x: f32,
    pub y: f32,
    pub delta_x: f32,
    pub delta_y: f32,
    pub button: WincastUnityPointerButton,
    pub key_code: i32,
    pub unicode_scalar: u32,
    pub timestamp_microseconds: u64,
}

impl Default for WincastUnityInputEvent {
    fn default() -> Self {
        Self {
            event_type: WincastUnityInputEventType::Unknown,
            pointer_id: 0,
            x: 0.0,
            y: 0.0,
            delta_x: 0.0,
            delta_y: 0.0,
            button: WincastUnityPointerButton::None,
            key_code: 0,
            unicode_scalar: 0,
            timestamp_microseconds: 0,
        }
    }
}

pub(crate) fn from_protocol_input(event: InputEvent) -> Option<WincastUnityInputEvent> {
    match event {
        InputEvent::MouseMove { x, y } | InputEvent::MouseMoveAbsolute { x, y } => {
            Some(WincastUnityInputEvent {
                event_type: WincastUnityInputEventType::PointerMove,
                x,
                y,
                ..Default::default()
            })
        }
        InputEvent::MouseButton { button, state } => Some(WincastUnityInputEvent {
            event_type: match state {
                ButtonState::Pressed => WincastUnityInputEventType::PointerDown,
                ButtonState::Released => WincastUnityInputEventType::PointerUp,
            },
            button: pointer_button_from_protocol(button),
            ..Default::default()
        }),
        InputEvent::MouseWheel { delta_x, delta_y } => Some(WincastUnityInputEvent {
            event_type: WincastUnityInputEventType::PointerScroll,
            delta_x: delta_x as f32,
            delta_y: delta_y as f32,
            ..Default::default()
        }),
        InputEvent::Key { code, state, .. } => Some(WincastUnityInputEvent {
            event_type: match state {
                ButtonState::Pressed => WincastUnityInputEventType::KeyDown,
                ButtonState::Released => WincastUnityInputEventType::KeyUp,
            },
            key_code: code as i32,
            ..Default::default()
        }),
        InputEvent::MouseMoveDelta { delta_x, delta_y } => Some(WincastUnityInputEvent {
            event_type: WincastUnityInputEventType::PointerMove,
            delta_x: delta_x as f32,
            delta_y: delta_y as f32,
            ..Default::default()
        }),
    }
}

fn pointer_button_from_protocol(button: MouseButton) -> WincastUnityPointerButton {
    match button {
        MouseButton::Left => WincastUnityPointerButton::Left,
        MouseButton::Right => WincastUnityPointerButton::Right,
        MouseButton::Middle => WincastUnityPointerButton::Middle,
    }
}

#[derive(Debug)]
pub(crate) struct InputQueue {
    events: VecDeque<WincastUnityInputEvent>,
    capacity: usize,
}

impl InputQueue {
    pub(crate) fn new() -> Self {
        Self {
            events: VecDeque::with_capacity(INPUT_QUEUE_CAPACITY),
            capacity: INPUT_QUEUE_CAPACITY,
        }
    }

    pub(crate) fn push(&mut self, event: WincastUnityInputEvent) -> UnityNativeResult<()> {
        if event.event_type == WincastUnityInputEventType::PointerMove
            && let Some(pending_move) = self
                .events
                .iter_mut()
                .rev()
                .find(|pending| pending.event_type == WincastUnityInputEventType::PointerMove)
        {
            *pending_move = event;
            return Ok(());
        }

        if self.events.len() >= self.capacity {
            if event.event_type == WincastUnityInputEventType::PointerMove {
                return Ok(());
            }

            if let Some(index) = self
                .events
                .iter()
                .position(|pending| pending.event_type == WincastUnityInputEventType::PointerMove)
            {
                self.events.remove(index);
            } else {
                return Err(UnityNativeError::InputQueueFull);
            }
        }

        self.events.push_back(event);
        Ok(())
    }

    pub(crate) fn drain_into(&mut self, output: &mut [WincastUnityInputEvent]) -> usize {
        let count = output.len().min(self.events.len());
        for slot in output.iter_mut().take(count) {
            *slot = self
                .events
                .pop_front()
                .expect("drain count should not exceed queue length");
        }
        count
    }
}
