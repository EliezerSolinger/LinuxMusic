//! The interface on the fake core (a real lm-doc document behind a fake engine): `cargo run -p lm-gui --example demo`.
//! `LM_LANG=en` shows the keys in English; `LM_PAGE=mixing` starts on the desk; `LM_OPEN=Settings,Routing` opens
//! screens at the start.

fn main() {
    lm_gui::run(|| Box::new(lm_port_fake::FakePort::new(Default::default()).expect("the demo project")));
}
