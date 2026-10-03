//! Host track metadata is read only on the main thread, outside the audio lock.
use super::*;
use clap_sys::ext::track_info::{
    CLAP_TRACK_INFO_HAS_TRACK_NAME, clap_host_track_info, clap_track_info,
};

impl<P: ClapPlugin> Wrapper<P> {
    pub(super) fn refresh_track_name(&self) {
        let Some(setup) = &self.setup else {
            return;
        };
        let extension = unsafe {
            query_host_extension::<clap_host_track_info>(&self.host_callback, CLAP_EXT_TRACK_INFO)
                .or_else(|| {
                    query_host_extension::<clap_host_track_info>(
                        &self.host_callback,
                        CLAP_EXT_TRACK_INFO_COMPAT,
                    )
                })
        };
        let mut info: clap_track_info = unsafe { mem::zeroed() };
        let available = extension.is_some_and(|extension| {
            extension.get.is_some_and(|get| unsafe { get(&*self.host_callback, &mut info) })
        });
        let name = if available && info.flags & CLAP_TRACK_INFO_HAS_TRACK_NAME != 0 {
            // Bound the read even if the host fills the complete fixed-size buffer.
            let bytes: Vec<u8> =
                info.name.iter().take_while(|&&c| c != 0).map(|&c| c as u8).collect();
            Some(String::from_utf8_lossy(&bytes).into_owned())
        } else {
            None
        };
        setup.track_name_changed(name.as_deref());
    }

    pub(super) unsafe extern "C" fn ext_track_info_changed(plugin: *const clap_plugin) {
        check_null_ptr!((), plugin, unsafe { (*plugin).plugin_data });
        let wrapper = unsafe { &*((*plugin).plugin_data as *const Self) };
        wrapper.refresh_track_name();
    }
}
