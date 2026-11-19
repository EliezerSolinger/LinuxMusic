//! Enumeration on Linux: it must work with no card at all (CI runners) and list the loopback card when it is there.
#![cfg(target_os = "linux")]
#![allow(clippy::print_stderr)]

use lm_sys_alsa::enumerate::{self, Probe};

#[test]
fn enumeration_works_without_cards() {
    let cards = enumerate::cards(true).expect("listing cards never fails for lack of cards");
    for card in &cards {
        assert!(!card.id.is_empty());
        for dev in &card.devices {
            assert!(dev.hw_name.starts_with("hw:CARD="));
            assert!(dev.playback.is_some() || dev.capture.is_some());
        }
    }
    eprintln!("{} card(s): {:?}", cards.len(), cards.iter().map(|c| &c.id).collect::<Vec<_>>());
}

#[test]
fn hints_include_the_null_device() {
    let hints = enumerate::pcm_hints().expect("hints");
    if std::env::var_os("LM_TEST_ALSA_NULL").is_some() {
        assert!(hints.iter().any(|h| h.name == "null"), "{hints:?}");
    }
}

#[test]
fn the_loopback_card_is_probed() {
    if std::env::var_os("LM_TEST_ALSA_LOOPBACK").is_none() {
        eprintln!("SKIPPED: no snd-aloop loopback card on this machine");
        return;
    }
    let cards = enumerate::cards(true).expect("cards");
    let card = cards.iter().find(|c| c.id == "Loopback").expect("the Loopback card is listed");
    let dev = card.devices.first().expect("a device");
    let Some(playback) = &dev.playback else { panic!("no playback direction") };
    match &playback.probe {
        Probe::Capabilities(c) => {
            assert!(c.channels_min >= 1 && c.channels_max >= c.channels_min);
            assert!(c.rates.contains(&48_000));
            assert!(!c.formats.is_empty());
        }
        other => panic!("probe: {other:?}"),
    }
    let caps = enumerate::probe(&dev.hw_name, lm_sys_alsa::Stream::Capture).expect("probe capture");
    assert!(caps.rate_max >= caps.rate_min);
}
