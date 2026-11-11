use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyboardState, SetKeyboardState, VK_CAPITAL, VK_CONTROL, VK_LSHIFT, VK_SHIFT};

use super::*;

#[test]
fn clear_key_state_turns_caps_lock_off_and_releases_modifiers() {
    let mut before = [0u8; 256];
    unsafe { GetKeyboardState(&mut before) }.unwrap();
    let mut set = before;
    set[VK_CAPITAL.0 as usize] |= 0x01;
    for vk in [VK_SHIFT, VK_LSHIFT, VK_CONTROL] {
        set[vk.0 as usize] |= 0x80;
    }
    unsafe { SetKeyboardState(&set) }.unwrap();
    let (caps_set, held_set) = (caps_lock_on(), held_modifiers());

    let cleared = clear_key_state();
    let (caps, held) = (caps_lock_on(), held_modifiers());
    let mut after = [0u8; 256];
    unsafe { GetKeyboardState(&mut after) }.unwrap();
    unsafe { SetKeyboardState(&before) }.unwrap();

    assert!(caps_set && held_set == ["Ctrl", "Shift"], "the test's own state was not set: caps {caps_set}, held {held_set:?}");
    cleared.unwrap();
    assert!(!caps, "Caps Lock still on in this thread's key state");
    assert!(held.is_empty(), "{held:?} still held in this thread's key state");
    assert_eq!(after[VK_LSHIFT.0 as usize] & 0x80, 0, "left Shift still down");
}
