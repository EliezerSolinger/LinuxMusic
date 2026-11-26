//! The pump returns on time with no window; a real window (`LM_GUI_TESTS=1`, a machine with a window system) can be
//! made, shown, resized, hidden and dropped

use std::time::{Duration, Instant};

#[test]
fn the_pump_returns_on_time() {
    let start = Instant::now();
    lm_sys_gui::pump(Duration::from_millis(30));
    let spent = start.elapsed();
    assert!(spent >= Duration::from_millis(20) && spent < Duration::from_secs(2), "{spent:?}");
}

#[test]
fn a_window_comes_and_goes() {
    if std::env::var_os("LM_GUI_TESTS").is_none() || !lm_sys_gui::supported() {
        return;
    }
    lm_sys_gui::init().unwrap();
    let mut w = lm_sys_gui::Window::new("test", 320, 200).unwrap();
    assert_ne!(w.handle(), 0);
    w.show();
    lm_sys_gui::pump(Duration::from_millis(50));
    w.set_size(400, 300);
    lm_sys_gui::pump(Duration::from_millis(50));
    assert!(!w.take_closed());
    w.hide();
    drop(w);
}
