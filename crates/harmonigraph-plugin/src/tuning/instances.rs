//! Main-thread instance discovery. Audio reads only the instance's atomics;
//! neither an open editor nor this registry is required for tuning.
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use harmonigraph_ui::params::{InstanceEdit, TuningInstance};

use super::{setup, DELAY_MULTIPLIER_MAX};

type Registry = Mutex<Vec<(u64, Weak<setup::Shared>)>>;

fn registry() -> &'static Registry {
    static INSTANCES: OnceLock<Registry> = OnceLock::new();
    INSTANCES.get_or_init(Default::default)
}

pub fn register(id: u64, shared: &Arc<setup::Shared>) {
    registry().lock().unwrap().push((id, Arc::downgrade(shared)));
}

pub fn retire(id: u64) {
    registry().lock().unwrap().retain(|(owner, _)| *owner != id);
}

pub fn snapshots() -> Vec<TuningInstance> {
    let mut instances: Vec<_> = registry()
        .lock()
        .unwrap()
        .iter()
        .filter_map(|(id, weak)| {
            let shared = weak.upgrade()?;
            let active = shared.active_multiplier.load(Ordering::Acquire);
            let requested = shared.requested_multiplier.load(Ordering::Acquire).max(1);
            let (rate, frames) = shared.format();
            let input = shared.last_input.load(Ordering::Relaxed);
            let output = shared.last_output.load(Ordering::Relaxed);
            let name = shared.name.lock().unwrap().clone();
            let misses = shared.misses.load(Ordering::Relaxed);
            let status = if active == 0 {
                "Waiting for host activation".to_owned()
            } else if misses != 0 {
                format!(
                    "{}\n{}",
                    super::session::status_text(shared.status()),
                    setup::deadline_text(shared.status(), misses)
                )
            } else {
                super::session::status_text(shared.status())
            };
            Some(TuningInstance {
                id: *id,
                name,
                is_hub: shared.is_hub(),
                retune: shared.retuning() & 1 != 0,
                show: shared.show.load(Ordering::Acquire),
                held: shared.held.load(Ordering::Relaxed),
                notes_in: shared.notes_in.load(Ordering::Relaxed),
                notes_out: shared.notes_out.load(Ordering::Relaxed),
                misses,
                delay: requested,
                max_delay: if shared.is_hub() { 1 } else { DELAY_MULTIPLIER_MAX as u32 },
                delay_text: setup::delay_text(requested, active, frames, rate),
                status,
                last_pitch: (input != i64::MIN && output != i64::MIN)
                    .then_some((input as f64 / 1_000_000.0, output as f64 / 1_000_000.0)),
            })
        })
        .collect();
    instances.sort_by_key(|instance| (!instance.is_hub, instance.id));
    instances
}

pub fn edit(id: u64, edit: InstanceEdit) {
    let shared = registry()
        .lock()
        .unwrap()
        .iter()
        .find(|(owner, _)| *owner == id)
        .and_then(|(_, weak)| weak.upgrade());
    if let Some(shared) = shared {
        match edit {
            InstanceEdit::Retune(value) => shared.set_retune(value),
            InstanceEdit::Show(value) => shared.set_show(value),
            InstanceEdit::Name(value) => shared.set_name(value),
            InstanceEdit::Delay(value) => shared.request_delay(value),
            InstanceEdit::Reset => shared.request_reset(),
        }
    }
}
