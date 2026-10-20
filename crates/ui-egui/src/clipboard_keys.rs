use egui::{Event, Key};

pub(crate) fn drop_duplicate_clipboard_keys(events: &mut Vec<Event>, web: bool) {
    if !web {
        return;
    }
    // Browser clipboard events already carry these shortcuts; synthetic keys are injected later.
    events.retain(|event| match event {
        Event::Key { key: Key::C | Key::X, modifiers, .. } => !modifiers.command,
        Event::Key { key: Key::V, modifiers, .. } => !modifiers.command || modifiers.shift,
        _ => true,
    });
}

pub(crate) fn ignore_paste_for_shift_v(events: &[Event], web: bool) -> bool {
    // Paste has no modifiers, so the key event must preserve Paste and Match Formatting.
    web && events.iter().any(|event| {
        matches!(event, Event::Key { key: Key::V, pressed: true, modifiers, .. } if modifiers.command && modifiers.shift)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Modifiers;

    fn key(key: Key, pressed: bool, modifiers: Modifiers) -> Event {
        Event::Key { key, physical_key: None, pressed, repeat: false, modifiers }
    }

    #[test]
    fn web_drops_clipboard_keys_but_keeps_clipboard_events() {
        for modifiers in [Modifiers::CTRL | Modifiers::COMMAND, Modifiers::MAC_CMD | Modifiers::COMMAND] {
            let clipboard = vec![Event::Paste("text".into()), Event::Copy, Event::Cut];
            let mut events = clipboard.clone();
            for k in [Key::V, Key::C, Key::X] {
                for pressed in [true, false] {
                    events.push(key(k, pressed, modifiers));
                    if k != Key::V {
                        events.push(key(k, pressed, modifiers | Modifiers::SHIFT));
                    }
                }
            }
            drop_duplicate_clipboard_keys(&mut events, true);
            assert_eq!(events, clipboard);
        }
    }

    #[test]
    fn web_keeps_other_keys_and_events() {
        let mut events = vec![Event::Text("v".into()), Event::PointerMoved(egui::pos2(10.0, 20.0))];
        for pressed in [true, false] {
            events.push(key(Key::Z, pressed, Modifiers::CTRL | Modifiers::COMMAND));
            events.push(key(Key::V, pressed, Modifiers::NONE));
            for k in [Key::C, Key::X, Key::V] {
                events.push(key(k, pressed, Modifiers::ALT));
                events.push(key(k, pressed, Modifiers::SHIFT));
            }
            events.push(key(Key::V, pressed, Modifiers::CTRL | Modifiers::COMMAND | Modifiers::SHIFT));
        }
        let original = events.clone();
        drop_duplicate_clipboard_keys(&mut events, true);
        assert_eq!(events, original);
    }

    #[test]
    fn desktop_keeps_all_clipboard_events_and_keys() {
        let mut events = vec![Event::Paste("text".into()), Event::Copy, Event::Cut];
        for k in [Key::V, Key::C, Key::X] {
            for pressed in [true, false] {
                for shift in [false, true] {
                    events.push(key(k, pressed, Modifiers { shift, ..Modifiers::CTRL | Modifiers::COMMAND }));
                }
            }
        }
        let original = events.clone();
        drop_duplicate_clipboard_keys(&mut events, false);
        assert_eq!(events, original);
        assert!(!ignore_paste_for_shift_v(&events, false));
    }

    #[test]
    fn shift_v_suppresses_paste_only_for_a_web_key_press() {
        let shift_v = key(Key::V, true, Modifiers::CTRL | Modifiers::COMMAND | Modifiers::SHIFT);
        for mut events in [vec![shift_v.clone(), Event::Paste("text".into())], vec![Event::Paste("text".into()), shift_v]] {
            drop_duplicate_clipboard_keys(&mut events, true);
            assert_eq!(events.len(), 2);
            assert!(ignore_paste_for_shift_v(&events, true));
        }
        for event in [
            key(Key::V, false, Modifiers::CTRL | Modifiers::COMMAND | Modifiers::SHIFT),
            key(Key::V, true, Modifiers::CTRL | Modifiers::COMMAND),
            key(Key::V, true, Modifiers::SHIFT),
            key(Key::C, true, Modifiers::CTRL | Modifiers::COMMAND | Modifiers::SHIFT),
            Event::Paste("text".into()),
        ] {
            assert!(!ignore_paste_for_shift_v(&[event], true));
        }
    }
}
