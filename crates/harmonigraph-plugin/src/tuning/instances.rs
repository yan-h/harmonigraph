//! Main-thread instance discovery. Audio reads only the instance's atomics;
//! neither an open editor nor this registry is required for tuning.
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use harmonigraph_ui::params::{InstanceEdit, TuningInstance, TuningVoice};

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
            let requested = setup::clamp_delay(shared.requested_multiplier.load(Ordering::Acquire));
            let (rate, frames) = shared.format();
            let input = shared.last_input.load(Ordering::Relaxed);
            let output = shared.last_output.load(Ordering::Relaxed);
            let name = shared.name.lock().unwrap().clone();
            let misses = shared.misses.load(Ordering::Relaxed);
            let bits = shared.status();
            let status = if frames == 0 {
                "Waiting for host activation".to_owned()
            } else {
                setup::detail_text(bits, misses)
            };
            Some(TuningInstance {
                id: *id,
                name,
                display_name: shared.display_name(),
                is_hub: shared.is_hub(),
                retune: shared.retuning() & 1 != 0,
                show: shared.show.load(Ordering::Acquire),
                voices: shared
                    .voices
                    .iter()
                    .filter_map(|voice| {
                        let voice = voice.load(Ordering::Relaxed);
                        (voice != u64::MAX).then_some(TuningVoice {
                            key: (voice >> 32) as u16,
                            pitch: f32::from_bits(voice as u32),
                        })
                    })
                    .collect(),
                notes_in: shared.notes_in.load(Ordering::Relaxed),
                notes_out: shared.notes_out.load(Ordering::Relaxed),
                misses,
                delay: if shared.is_hub() { 0 } else { requested },
                max_delay: if shared.is_hub() { 0 } else { DELAY_MULTIPLIER_MAX as u32 },
                delay_text: if shared.is_hub() {
                    "No tuning delay".to_owned()
                } else {
                    setup::delay_text(requested, active, frames, rate)
                },
                status,
                // Waiting for activation is a status to inspect too (Yan,
                // #1861), and misses stay red until Reset (#1798 H3).
                faulted: frames == 0 || bits != 0 || misses != 0,
                last_pitch: (input != i64::MIN && output != i64::MIN)
                    .then_some((input as f64 / 1_000_000.0, output as f64 / 1_000_000.0)),
            })
        })
        .collect();
    instances.sort_by_key(|instance| (!instance.is_hub, instance.id));
    instances
}

/// An edit to the instance `id`, or for Reset all voices, which belongs to the
/// session rather than to any row, to the session.
pub fn edit(id: u64, edit: InstanceEdit) {
    if let InstanceEdit::Reset = edit {
        super::session::session().request_reset();
        return;
    }
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
            InstanceEdit::Reset => unreachable!("handled above"),
        }
    }
}
