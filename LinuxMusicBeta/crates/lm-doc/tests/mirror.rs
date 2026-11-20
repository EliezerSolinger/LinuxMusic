//! A mirror (I-15, the RemotePort): another document's roots and diffs, delivered to its own feeds as they are

mod common;

use common::*;
use lm_doc::{Document, Options, Topic};

fn mirror_of(doc: &Document) -> Document {
    let opts = Options { history_limit: 0, journal: false, rules: Vec::new(), ..options() };
    Document::from_root(registry(), lm_doc::PROJECT, doc.root().clone(), opts).unwrap().document
}

#[test]
fn a_mirror_gives_the_same_diffs() {
    let mut doc = new_doc();
    let mut mirror = mirror_of(&doc);
    let mut all = doc.feed(Topic::All);
    let mut tracks = doc.feed(Topic::Type("track"));
    let mut m_all = mirror.feed(Topic::All);
    let mut m_tracks = mirror.feed(Topic::Type("track"));
    let t = add_track(&mut doc, "a", Some(synth()));
    add_item(&mut doc, t, "midi");
    // The original's merged diff since the feed was made, shipped as one change
    let d_all = all.drain(&doc).unwrap();
    let d_tracks = tracks.drain(&doc).unwrap();
    mirror.mirror(doc.root().clone(), Some(&d_all)).unwrap();
    assert_eq!(mirror.root(), doc.root());
    assert_eq!(m_all.drain(&mirror).unwrap(), d_all);
    assert_eq!(m_tracks.drain(&mirror).unwrap(), d_tracks);
    // Two changes shipped one by one merge in the mirror's feed as in the original's
    doc.transact(label("t.vol"), |e| e.set_param(t, "volume", 0.5)).unwrap();
    let d1 = all.drain(&doc).unwrap();
    mirror.mirror(doc.root().clone(), Some(&d1)).unwrap();
    doc.transact(label("t.name"), |e| e.set(t, &[k("name")], text("b"))).unwrap();
    let d2 = all.drain(&doc).unwrap();
    mirror.mirror(doc.root().clone(), Some(&d2)).unwrap();
    let both = m_all.drain(&mirror).unwrap();
    assert_eq!(both.changed.len(), 1);
    assert_eq!(both.changed[0].paths.len(), 2);
}

#[test]
fn a_resync_works_out_the_diff() {
    let mut doc = new_doc();
    let t1 = add_track(&mut doc, "a", None);
    let mut mirror = mirror_of(&doc);
    let mut feed = mirror.feed(Topic::All);
    let t2 = add_track(&mut doc, "b", None);
    doc.transact(label("t.rm"), |e| e.remove(t1)).unwrap();
    let ch = add_channel(&mut doc, "c");
    let d = mirror.mirror(doc.root().clone(), None).unwrap();
    assert!(d.created.contains(&t2) && d.created.contains(&ch));
    assert!(d.removed.contains(&t1));
    assert_eq!(feed.drain(&mirror).unwrap(), d);
    assert_eq!(mirror.root(), doc.root());
    assert_eq!(mirror.find(t2), doc.find(t2));
}
