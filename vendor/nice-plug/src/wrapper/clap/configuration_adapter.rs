//! The configuration walk, sharing the owned input pool with the independently
//! opted-in performance boundary. The plugin owns its musical configuration and
//! its own edit queue; the wrapper only hands it the callback's boundary, then
//! every owned input in order with its exact sample, then the group's end.
use super::*;
use crate::wrapper::clap::configuration::*;

impl<P: ClapPlugin> Wrapper<P> {
    pub(super) fn process_configuration(&self, boundary: ConfigurationBoundary) {
        let mut input_guard = self.owned_input.lock();
        let input = input_guard.as_mut().unwrap();
        let mut plugin = self.plugin.lock();
        // The block's first configuration, once the plugin has reduced the
        // edits queued before this callback: what every group sees until
        // automation changes it.
        plugin.clap_configuration_begin(boundary);
        input.storage.bind_untimed(boundary.steady_time);
        // One walk, in input order, with each event's sample-precise timestamp
        // preserved.
        while let Some(event) = input.get(0) {
            plugin.clap_configuration_observe(event);
            input.ack_configuration(P::CLAP_PERFORMANCE);
        }
        // Learning reads the whole block's evidence and lands at the next
        // boundary with everything else.
        plugin.clap_configuration_group_end(boundary.steady_time);
    }
}
