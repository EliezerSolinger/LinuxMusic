//! The C declarations: lilv's exported functions, the public `LilvInstance` struct (lilv's `lilv_instance_*` calls are
//! `static inline` in lilv.h, so they are written here from that struct), and the LV2 structs the host provides or
//! receives. Written by hand from lilv.h (0.24.10 to 0.28) and the LV2 1.18 headers; all are stable ABI.

#![allow(non_camel_case_types, dead_code)]

use std::ffi::{c_char, c_double, c_float, c_int, c_uint, c_void};

macro_rules! opaque {
    ($($name:ident),*) => {
        $(
            #[repr(C)]
            pub struct $name {
                _private: [u8; 0],
            }
        )*
    };
}

opaque!(LilvWorld, LilvPlugin, LilvPluginClass, LilvPort, LilvNode, LilvPlugins, LilvNodes, LilvIter);

pub type LV2_Handle = *mut c_void;
pub type LV2_URID = u32;

/// `LV2_Feature`
#[repr(C)]
pub struct LV2_Feature {
    pub uri: *const c_char,
    pub data: *mut c_void,
}

/// `LV2_Descriptor`
#[repr(C)]
pub struct LV2_Descriptor {
    pub uri: *const c_char,
    pub instantiate: Option<
        unsafe extern "C" fn(*const LV2_Descriptor, c_double, *const c_char, *const *const LV2_Feature) -> LV2_Handle,
    >,
    pub connect_port: Option<unsafe extern "C" fn(LV2_Handle, u32, *mut c_void)>,
    pub activate: Option<unsafe extern "C" fn(LV2_Handle)>,
    pub run: Option<unsafe extern "C" fn(LV2_Handle, u32)>,
    pub deactivate: Option<unsafe extern "C" fn(LV2_Handle)>,
    pub cleanup: Option<unsafe extern "C" fn(LV2_Handle)>,
    pub extension_data: Option<unsafe extern "C" fn(*const c_char) -> *const c_void>,
}

/// `struct LilvInstanceImpl` (public in lilv.h)
#[repr(C)]
pub struct LilvInstance {
    pub lv2_descriptor: *const LV2_Descriptor,
    pub lv2_handle: LV2_Handle,
    pub pimpl: *mut c_void,
}

/// `LV2_URID_Map`
#[repr(C)]
pub struct LV2_URID_Map {
    pub handle: *mut c_void,
    pub map: unsafe extern "C" fn(*mut c_void, *const c_char) -> LV2_URID,
}

/// `LV2_URID_Unmap`
#[repr(C)]
pub struct LV2_URID_Unmap {
    pub handle: *mut c_void,
    pub unmap: unsafe extern "C" fn(*mut c_void, LV2_URID) -> *const c_char,
}

pub const LV2_OPTIONS_INSTANCE: c_uint = 0;

/// `LV2_Options_Option`
#[repr(C)]
pub struct LV2_Options_Option {
    pub context: c_uint,
    pub subject: u32,
    pub key: LV2_URID,
    pub size: u32,
    pub type_: LV2_URID,
    pub value: *const c_void,
}

pub const LV2_WORKER_SUCCESS: c_int = 0;
pub const LV2_WORKER_ERR_UNKNOWN: c_int = 1;
pub const LV2_WORKER_ERR_NO_SPACE: c_int = 2;

pub type LV2_Worker_Respond_Function = unsafe extern "C" fn(*mut c_void, u32, *const c_void) -> c_int;

/// `LV2_Worker_Interface`
#[repr(C)]
pub struct LV2_Worker_Interface {
    pub work:
        Option<unsafe extern "C" fn(LV2_Handle, LV2_Worker_Respond_Function, *mut c_void, u32, *const c_void) -> c_int>,
    pub work_response: Option<unsafe extern "C" fn(LV2_Handle, u32, *const c_void) -> c_int>,
    pub end_run: Option<unsafe extern "C" fn(LV2_Handle) -> c_int>,
}

/// `LV2_Worker_Schedule`
#[repr(C)]
pub struct LV2_Worker_Schedule {
    pub handle: *mut c_void,
    pub schedule_work: unsafe extern "C" fn(*mut c_void, u32, *const c_void) -> c_int,
}

pub const LV2_STATE_IS_POD: u32 = 1;
pub const LV2_STATE_IS_PORTABLE: u32 = 2;
pub const LV2_STATE_SUCCESS: c_int = 0;
pub const LV2_STATE_ERR_UNKNOWN: c_int = 1;
pub const LV2_STATE_ERR_BAD_FLAGS: c_int = 3;

pub type LV2_State_Store_Function = unsafe extern "C" fn(*mut c_void, u32, *const c_void, usize, u32, u32) -> c_int;
pub type LV2_State_Retrieve_Function =
    unsafe extern "C" fn(*mut c_void, u32, *mut usize, *mut u32, *mut u32) -> *const c_void;

/// `LV2_State_Interface`
#[repr(C)]
pub struct LV2_State_Interface {
    pub save: Option<
        unsafe extern "C" fn(
            LV2_Handle,
            LV2_State_Store_Function,
            *mut c_void,
            u32,
            *const *const LV2_Feature,
        ) -> c_int,
    >,
    pub restore: Option<
        unsafe extern "C" fn(
            LV2_Handle,
            LV2_State_Retrieve_Function,
            *mut c_void,
            u32,
            *const *const LV2_Feature,
        ) -> c_int,
    >,
}

/// `LV2_State_Map_Path`
#[repr(C)]
pub struct LV2_State_Map_Path {
    pub handle: *mut c_void,
    pub abstract_path: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_char,
    pub absolute_path: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_char,
}

/// `LV2_State_Free_Path`
#[repr(C)]
pub struct LV2_State_Free_Path {
    pub handle: *mut c_void,
    pub free_path: unsafe extern "C" fn(*mut c_void, *mut c_char),
}

unsafe extern "C" {
    // The C library: a path handed to a plugin through state:mapPath is a `malloc`ed copy it frees with `free` (or
    // through state:freePath, which calls `free`)
    pub fn strdup(s: *const c_char) -> *mut c_char;
    pub fn free(p: *mut c_void);

    pub safe fn lilv_world_new() -> *mut LilvWorld;
    pub fn lilv_world_free(world: *mut LilvWorld);
    pub fn lilv_world_set_option(world: *mut LilvWorld, uri: *const c_char, value: *const LilvNode);
    pub fn lilv_world_load_all(world: *mut LilvWorld);
    pub fn lilv_world_load_bundle(world: *mut LilvWorld, bundle_uri: *const LilvNode);
    pub fn lilv_world_get_all_plugins(world: *const LilvWorld) -> *const LilvPlugins;

    pub fn lilv_new_uri(world: *mut LilvWorld, uri: *const c_char) -> *mut LilvNode;
    pub fn lilv_new_string(world: *mut LilvWorld, s: *const c_char) -> *mut LilvNode;
    pub fn lilv_new_bool(world: *mut LilvWorld, val: bool) -> *mut LilvNode;
    pub fn lilv_new_file_uri(world: *mut LilvWorld, host: *const c_char, path: *const c_char) -> *mut LilvNode;
    pub fn lilv_node_free(node: *mut LilvNode);
    pub fn lilv_node_as_string(node: *const LilvNode) -> *const c_char;
    pub fn lilv_node_as_uri(node: *const LilvNode) -> *const c_char;
    pub fn lilv_node_is_uri(node: *const LilvNode) -> bool;
    pub fn lilv_node_is_float(node: *const LilvNode) -> bool;
    pub fn lilv_node_is_int(node: *const LilvNode) -> bool;
    pub fn lilv_node_is_bool(node: *const LilvNode) -> bool;
    pub fn lilv_node_as_float(node: *const LilvNode) -> c_float;
    pub fn lilv_node_as_int(node: *const LilvNode) -> c_int;
    pub fn lilv_node_as_bool(node: *const LilvNode) -> bool;
    pub fn lilv_file_uri_parse(uri: *const c_char, hostname: *mut *mut c_char) -> *mut c_char;
    pub fn lilv_free(ptr: *mut c_void);

    pub fn lilv_plugins_size(plugins: *const LilvPlugins) -> c_uint;
    pub fn lilv_plugins_begin(plugins: *const LilvPlugins) -> *mut LilvIter;
    pub fn lilv_plugins_next(plugins: *const LilvPlugins, i: *mut LilvIter) -> *mut LilvIter;
    pub fn lilv_plugins_is_end(plugins: *const LilvPlugins, i: *const LilvIter) -> bool;
    pub fn lilv_plugins_get(plugins: *const LilvPlugins, i: *const LilvIter) -> *const LilvPlugin;
    pub fn lilv_plugins_get_by_uri(plugins: *const LilvPlugins, uri: *const LilvNode) -> *const LilvPlugin;

    pub fn lilv_nodes_size(nodes: *const LilvNodes) -> c_uint;
    pub fn lilv_nodes_begin(nodes: *const LilvNodes) -> *mut LilvIter;
    pub fn lilv_nodes_next(nodes: *const LilvNodes, i: *mut LilvIter) -> *mut LilvIter;
    pub fn lilv_nodes_is_end(nodes: *const LilvNodes, i: *const LilvIter) -> bool;
    pub fn lilv_nodes_get(nodes: *const LilvNodes, i: *const LilvIter) -> *const LilvNode;
    pub fn lilv_nodes_free(nodes: *mut LilvNodes);

    pub fn lilv_plugin_verify(plugin: *const LilvPlugin) -> bool;
    pub fn lilv_plugin_get_uri(plugin: *const LilvPlugin) -> *const LilvNode;
    pub fn lilv_plugin_get_bundle_uri(plugin: *const LilvPlugin) -> *const LilvNode;
    pub fn lilv_plugin_get_library_uri(plugin: *const LilvPlugin) -> *const LilvNode;
    pub fn lilv_plugin_get_name(plugin: *const LilvPlugin) -> *mut LilvNode;
    pub fn lilv_plugin_get_author_name(plugin: *const LilvPlugin) -> *mut LilvNode;
    pub fn lilv_plugin_get_class(plugin: *const LilvPlugin) -> *const LilvPluginClass;
    pub fn lilv_plugin_class_get_uri(class: *const LilvPluginClass) -> *const LilvNode;
    pub fn lilv_plugin_class_get_label(class: *const LilvPluginClass) -> *const LilvNode;
    pub fn lilv_plugin_get_value(plugin: *const LilvPlugin, predicate: *const LilvNode) -> *mut LilvNodes;
    pub fn lilv_plugin_get_required_features(plugin: *const LilvPlugin) -> *mut LilvNodes;
    pub fn lilv_plugin_get_optional_features(plugin: *const LilvPlugin) -> *mut LilvNodes;
    pub fn lilv_plugin_get_extension_data(plugin: *const LilvPlugin) -> *mut LilvNodes;
    pub fn lilv_plugin_get_num_ports(plugin: *const LilvPlugin) -> u32;
    pub fn lilv_plugin_get_port_by_index(plugin: *const LilvPlugin, index: u32) -> *const LilvPort;
    pub fn lilv_plugin_has_latency(plugin: *const LilvPlugin) -> bool;
    pub fn lilv_plugin_get_latency_port_index(plugin: *const LilvPlugin) -> u32;
    pub fn lilv_plugin_instantiate(
        plugin: *const LilvPlugin,
        sample_rate: c_double,
        features: *const *const LV2_Feature,
    ) -> *mut LilvInstance;
    pub fn lilv_instance_free(instance: *mut LilvInstance);

    pub fn lilv_port_get_symbol(plugin: *const LilvPlugin, port: *const LilvPort) -> *const LilvNode;
    pub fn lilv_port_get_name(plugin: *const LilvPlugin, port: *const LilvPort) -> *mut LilvNode;
    pub fn lilv_port_is_a(plugin: *const LilvPlugin, port: *const LilvPort, class: *const LilvNode) -> bool;
    pub fn lilv_port_has_property(plugin: *const LilvPlugin, port: *const LilvPort, property: *const LilvNode) -> bool;
    pub fn lilv_port_supports_event(plugin: *const LilvPlugin, port: *const LilvPort, event: *const LilvNode) -> bool;
    pub fn lilv_port_get(plugin: *const LilvPlugin, port: *const LilvPort, predicate: *const LilvNode)
    -> *mut LilvNode;
    pub fn lilv_port_get_range(
        plugin: *const LilvPlugin,
        port: *const LilvPort,
        def: *mut *mut LilvNode,
        min: *mut *mut LilvNode,
        max: *mut *mut LilvNode,
    );
}
