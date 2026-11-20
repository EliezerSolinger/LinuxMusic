//! The hand-written ABI against the C compiler's: every struct's size, alignment and field offsets, compiled from
//! the vendored CLAP headers

use std::mem::{align_of, offset_of, size_of};
use std::process::Command;

use lm_sys_clap::layout::*;

macro_rules! rust {
    ($t:ident { $($f:ident),* }) => {{
        let mut v = vec![
            format!("{} size {}", stringify!($t), size_of::<$t>()),
            format!("{} align {}", stringify!($t), align_of::<$t>()),
        ];
        $( v.push(format!("{} {} {}", stringify!($t), stringify!($f), offset_of!($t, $f))); )*
        v
    }};
}

/// (Rust name, C type, [(Rust field, C field)])
fn c_lines(name: &str, c: &str, fields: &[(&str, &str)]) -> String {
    let mut s =
        format!("printf(\"{name} size %zu\\n\", sizeof({c}));\nprintf(\"{name} align %zu\\n\", _Alignof({c}));\n");
    for (r, f) in fields {
        s += &format!("printf(\"{name} {r} %zu\\n\", offsetof({c}, {f}));\n");
    }
    s
}

#[test]
fn every_struct_matches_the_headers() {
    let mut want: Vec<String> = Vec::new();
    let mut c = String::new();
    macro_rules! check {
        ($t:ident, $ct:expr, { $($f:ident => $cf:expr),* }) => {{
            want.extend(rust!($t { $($f),* }));
            c += &c_lines(stringify!($t), $ct, &[$((stringify!($f), $cf)),*]);
        }};
    }
    check!(clap_version, "clap_version_t", { major => "major", minor => "minor", revision => "revision" });
    check!(clap_plugin_entry, "clap_plugin_entry_t", { clap_version => "clap_version", init => "init", deinit => "deinit", get_factory => "get_factory" });
    check!(clap_plugin_factory, "clap_plugin_factory_t", { get_plugin_count => "get_plugin_count", get_plugin_descriptor => "get_plugin_descriptor", create_plugin => "create_plugin" });
    check!(clap_plugin_descriptor, "clap_plugin_descriptor_t", { id => "id", name => "name", vendor => "vendor", url => "url", manual_url => "manual_url", support_url => "support_url", version => "version", description => "description", features => "features" });
    check!(clap_plugin, "clap_plugin_t", { desc => "desc", plugin_data => "plugin_data", init => "init", destroy => "destroy", activate => "activate", deactivate => "deactivate", start_processing => "start_processing", stop_processing => "stop_processing", reset => "reset", process => "process", get_extension => "get_extension", on_main_thread => "on_main_thread" });
    check!(clap_host, "clap_host_t", { clap_version => "clap_version", host_data => "host_data", name => "name", vendor => "vendor", url => "url", version => "version", get_extension => "get_extension", request_restart => "request_restart", request_process => "request_process", request_callback => "request_callback" });
    check!(clap_audio_buffer, "clap_audio_buffer_t", { data32 => "data32", data64 => "data64", channel_count => "channel_count", latency => "latency", constant_mask => "constant_mask" });
    check!(clap_process, "clap_process_t", { steady_time => "steady_time", frames_count => "frames_count", transport => "transport", audio_inputs => "audio_inputs", audio_outputs => "audio_outputs", audio_inputs_count => "audio_inputs_count", audio_outputs_count => "audio_outputs_count", in_events => "in_events", out_events => "out_events" });
    check!(clap_event_header, "clap_event_header_t", { size => "size", time => "time", space_id => "space_id", type_ => "type", flags => "flags" });
    check!(clap_event_note, "clap_event_note_t", { header => "header", note_id => "note_id", port_index => "port_index", channel => "channel", key => "key", velocity => "velocity" });
    check!(clap_event_note_expression, "clap_event_note_expression_t", { expression_id => "expression_id", note_id => "note_id", port_index => "port_index", channel => "channel", key => "key", value => "value" });
    check!(clap_event_param_value, "clap_event_param_value_t", { param_id => "param_id", cookie => "cookie", note_id => "note_id", port_index => "port_index", channel => "channel", key => "key", value => "value" });
    check!(clap_event_param_gesture, "clap_event_param_gesture_t", { param_id => "param_id" });
    check!(clap_event_transport, "clap_event_transport_t", { flags => "flags", song_pos_beats => "song_pos_beats", song_pos_seconds => "song_pos_seconds", tempo => "tempo", tempo_inc => "tempo_inc", loop_start_beats => "loop_start_beats", loop_end_beats => "loop_end_beats", loop_start_seconds => "loop_start_seconds", loop_end_seconds => "loop_end_seconds", bar_start => "bar_start", bar_number => "bar_number", tsig_num => "tsig_num", tsig_denom => "tsig_denom" });
    check!(clap_event_midi, "clap_event_midi_t", { port_index => "port_index", data => "data" });
    check!(clap_input_events, "clap_input_events_t", { ctx => "ctx", size => "size", get => "get" });
    check!(clap_output_events, "clap_output_events_t", { ctx => "ctx", try_push => "try_push" });
    check!(clap_param_info, "clap_param_info_t", { id => "id", flags => "flags", cookie => "cookie", name => "name", module => "module", min_value => "min_value", max_value => "max_value", default_value => "default_value" });
    check!(clap_plugin_params, "clap_plugin_params_t", { count => "count", get_info => "get_info", get_value => "get_value", value_to_text => "value_to_text", text_to_value => "text_to_value", flush => "flush" });
    check!(clap_host_params, "clap_host_params_t", { rescan => "rescan", clear => "clear", request_flush => "request_flush" });
    check!(clap_plugin_latency, "clap_plugin_latency_t", { get => "get" });
    check!(clap_host_latency, "clap_host_latency_t", { changed => "changed" });
    check!(clap_istream, "clap_istream_t", { ctx => "ctx", read => "read" });
    check!(clap_ostream, "clap_ostream_t", { ctx => "ctx", write => "write" });
    check!(clap_plugin_state, "clap_plugin_state_t", { save => "save", load => "load" });
    check!(clap_host_state, "clap_host_state_t", { mark_dirty => "mark_dirty" });
    check!(clap_audio_port_info, "clap_audio_port_info_t", { id => "id", name => "name", flags => "flags", channel_count => "channel_count", port_type => "port_type", in_place_pair => "in_place_pair" });
    check!(clap_plugin_audio_ports, "clap_plugin_audio_ports_t", { count => "count", get => "get" });
    check!(clap_host_audio_ports, "clap_host_audio_ports_t", { is_rescan_flag_supported => "is_rescan_flag_supported", rescan => "rescan" });
    check!(clap_note_port_info, "clap_note_port_info_t", { id => "id", supported_dialects => "supported_dialects", preferred_dialect => "preferred_dialect", name => "name" });
    check!(clap_plugin_note_ports, "clap_plugin_note_ports_t", { count => "count", get => "get" });
    check!(clap_host_note_ports, "clap_host_note_ports_t", { supported_dialects => "supported_dialects", rescan => "rescan" });
    check!(clap_host_log, "clap_host_log_t", { log => "log" });
    check!(clap_host_thread_check, "clap_host_thread_check_t", { is_main_thread => "is_main_thread", is_audio_thread => "is_audio_thread" });
    check!(clap_window, "clap_window_t", { api => "api", handle => "cocoa" });
    check!(clap_plugin_gui, "clap_plugin_gui_t", { is_api_supported => "is_api_supported", get_preferred_api => "get_preferred_api", create => "create", destroy => "destroy", set_scale => "set_scale", get_size => "get_size", can_resize => "can_resize", get_resize_hints => "get_resize_hints", adjust_size => "adjust_size", set_size => "set_size", set_parent => "set_parent", set_transient => "set_transient", suggest_title => "suggest_title", show => "show", hide => "hide" });
    check!(clap_host_gui, "clap_host_gui_t", { resize_hints_changed => "resize_hints_changed", request_resize => "request_resize", request_show => "request_show", request_hide => "request_hide", closed => "closed" });

    let program = format!(
        "#include <stdio.h>\n#include <stddef.h>\n#include <clap/clap.h>\nint main(void) {{\n{c}return 0;\n}}\n"
    );
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("clap-layout-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("layout.c");
    let exe = dir.join("layout");
    std::fs::write(&src, program).unwrap();
    let include = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("clap/include");
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".into());
    let ok = Command::new(cc).arg("-std=c11").arg("-I").arg(&include).arg("-o").arg(&exe).arg(&src).status().unwrap();
    assert!(ok.success(), "the layout program compiles");
    let out = Command::new(&exe).output().unwrap();
    let got: Vec<String> = String::from_utf8(out.stdout).unwrap().lines().map(str::to_owned).collect();
    let _ = std::fs::remove_dir_all(&dir);
    let diff: Vec<(String, String)> =
        want.iter().zip(&got).filter(|(w, g)| w != g).map(|(w, g)| (w.clone(), g.clone())).collect();
    assert_eq!(want.len(), got.len());
    assert!(diff.is_empty(), "Rust vs C: {diff:#?}");
}
