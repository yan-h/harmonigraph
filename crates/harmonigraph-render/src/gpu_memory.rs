//! The GPU memory record: every texture and buffer this crate allocates, with
//! its label, shape and the bytes the driver actually gave it, for as long as
//! it is alive.
//!
//! Every allocation in the crate goes through [`texture`], [`buffer`] or
//! [`buffer_init`], and `clippy.toml` refuses the `wgpu` calls they wrap, so a
//! new target cannot be added without landing here.
//!
//! What is alive is the driver's answer, not ours. Each entry holds a WEAK
//! reference to the Metal object underneath, which dies only when Metal frees
//! it — after wgpu has also let go of it, so a texture still held by a cached
//! view or bind group, or by a frame in flight, still counts. A record that
//! dropped an entry when our handle dropped would read low exactly when it
//! matters: across a style switch, while the old targets wait out the frame
//! still using them.
//!
//! The bytes are Metal's `allocatedSize`, which includes the padding and
//! alignment the texture's layout costs, rather than width × height × format.
//!
//! What it does not hold is everything this crate did not allocate: egui's
//! own textures and buffers, wgpu's internals and the swapchain, which
//! [`Totals::device`] counts, and the driver's own pool, which nothing here
//! sees. `docs/gpu-memory.md` reads the record against `footprint`.
//!
//! [`snapshot`] lists what is alive; [`Totals`] is the summary the HUD reads
//! once a second. Only on Metal: elsewhere nothing is recorded and both say
//! so.

use crate::wgpu;

/// One live allocation.
#[derive(Clone, Debug, PartialEq)]
pub struct Resource {
    pub label: String,
    pub shape: Shape,
    /// What Metal allocated for it, padding included.
    pub bytes: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    Texture { size: [u32; 3], mips: u32, samples: u32, format: wgpu::TextureFormat },
    Buffer { size: u64 },
}

/// The record's headline numbers.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Totals {
    /// Bytes held by live recorded resources.
    pub live: u64,
    /// The most `live` has ever been. Exact, because the total can only rise
    /// at an allocation and every allocation re-counts it.
    pub peak: u64,
    /// What every Metal device the record has seen holds in all — ours,
    /// egui's and wgpu's own, and anything else in the process using the same
    /// GPU, such as another plugin in the host. `None` until something is
    /// recorded on Metal.
    pub device: Option<u64>,
}

/// Every live recorded resource, largest first, and the totals.
#[derive(Clone, Debug, Default)]
pub struct GpuMemory {
    pub resources: Vec<Resource>,
    pub totals: Totals,
    /// What was alive at the peak, which is what explains it: the peak is
    /// the steady set plus whatever was still alive beside it then.
    pub at_peak: Vec<Resource>,
}

impl GpuMemory {
    /// The record as a table, one row per label and shape, largest first,
    /// then what the peak held that is gone now.
    pub fn table(&self) -> String {
        let t = &self.totals;
        let mut out = format!("GPU memory: {} live, peak {}", mib(t.live), mib(t.peak));
        if let Some(device) = t.device {
            let other = device.saturating_sub(t.live);
            out += &format!(", device {} ({} not recorded here)", mib(device), mib(other));
        }
        out += "\n";
        out += &table(&self.resources);
        let mut gone = self.at_peak.clone();
        for r in &self.resources {
            if let Some(at) = gone.iter().position(|g| g == r) {
                gone.swap_remove(at);
            }
        }
        if !gone.is_empty() {
            // Entries are kept in allocation order, so the last is the one
            // whose allocation set the peak.
            let by = self.at_peak.last().map_or("", |r| r.label.as_str());
            out += &format!("Alive at the peak, which allocating {by} set, and freed since:\n");
            out += &table(&gone);
        }
        out
    }
}

/// `resources` as table rows, one per label and shape, largest first.
fn table(resources: &[Resource]) -> String {
    let mut rows: Vec<(&str, &Shape, u32, u64)> = Vec::new();
    for r in resources {
        match rows.iter_mut().find(|(label, shape, ..)| *label == r.label && *shape == &r.shape) {
            Some(row) => {
                row.2 += 1;
                row.3 += r.bytes;
            }
            None => rows.push((&r.label, &r.shape, 1, r.bytes)),
        }
    }
    rows.sort_by(|a, b| b.3.cmp(&a.3).then(a.0.cmp(b.0)));
    let mut out = "| MB | n | label | shape |\n|---:|---:|---|---|\n".to_owned();
    for (label, shape, n, bytes) in rows {
        out += &format!("| {:.1} | {n} | {label} | {} |\n", bytes as f64 / MIB, describe(shape));
    }
    out
}

const MIB: f64 = 1024.0 * 1024.0;

fn mib(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / MIB)
}

fn describe(shape: &Shape) -> String {
    match *shape {
        Shape::Texture { size: [w, h, d], mips, samples, format } => {
            let mut s = format!("{w}×{h}");
            if d > 1 {
                s += &format!("×{d}");
            }
            s += &format!(" {format:?}");
            if mips > 1 {
                s += &format!(" {mips} mips");
            }
            if samples > 1 {
                s += &format!(" {samples}× MSAA");
            }
            s
        }
        Shape::Buffer { size } => format!("buffer {size} B"),
    }
}

/// `device.create_texture`, recorded.
#[allow(clippy::disallowed_methods)]
pub(crate) fn texture(device: &wgpu::Device, desc: &wgpu::TextureDescriptor) -> wgpu::Texture {
    let texture = device.create_texture(desc);
    let shape = Shape::Texture {
        size: [desc.size.width, desc.size.height, desc.size.depth_or_array_layers],
        mips: desc.mip_level_count,
        samples: desc.sample_count,
        format: desc.format,
    };
    metal::record(device, desc.label, shape, metal::texture(&texture));
    texture
}

/// `device.create_buffer`, recorded.
#[allow(clippy::disallowed_methods)]
pub(crate) fn buffer(device: &wgpu::Device, desc: &wgpu::BufferDescriptor) -> wgpu::Buffer {
    let buffer = device.create_buffer(desc);
    metal::record(device, desc.label, Shape::Buffer { size: desc.size }, metal::buffer(&buffer));
    buffer
}

/// `device.create_buffer_init`, recorded.
#[allow(clippy::disallowed_methods)]
pub(crate) fn buffer_init(
    device: &wgpu::Device,
    desc: &wgpu::util::BufferInitDescriptor,
) -> wgpu::Buffer {
    use wgpu::util::DeviceExt;
    let buffer = device.create_buffer_init(desc);
    let shape = Shape::Buffer { size: buffer.size() };
    metal::record(device, desc.label, shape, metal::buffer(&buffer));
    buffer
}

/// Every live recorded resource, largest first, across every device.
pub fn snapshot() -> GpuMemory {
    metal::snapshot(None)
}

/// The headline numbers alone.
pub fn totals() -> Totals {
    metal::totals()
}

/// What one device holds, and the most it has held. Tests share a process
/// with other tests' devices, so they read their own.
#[cfg(all(any(test, feature = "test-support"), target_vendor = "apple"))]
pub fn snapshot_of(device: &wgpu::Device) -> GpuMemory {
    metal::snapshot(Some(device))
}

#[cfg(target_vendor = "apple")]
mod metal {
    use std::sync::Mutex;

    use crate::wgpu::{self, hal::api::Metal};
    use objc2::rc::{Retained, Weak};
    use objc2::runtime::ProtocolObject;
    use objc2::Message;
    use objc2_metal::{MTLDevice, MTLResource};

    use super::{GpuMemory, Resource, Shape, Totals};

    /// A weak reference the record can keep in a static. Metal objects are
    /// thread-safe, and so is the runtime's weak table.
    struct Handle<T: ?Sized + Message>(Weak<T>);
    unsafe impl<T: ?Sized + Message> Send for Handle<T> {}

    /// Which wgpu device made an allocation: the address of its Metal backend
    /// device, which wgpu keeps in place for the device's life. Not the
    /// `MTLDevice`, which is the one system GPU every wgpu device shares.
    type Owner = usize;

    struct Entry {
        owner: Owner,
        resource: Resource,
        handle: Handle<ProtocolObject<dyn MTLResource>>,
    }

    /// The most everyone (`None`) has held, and what; in tests, each owner's
    /// too.
    struct Peak {
        owner: Option<Owner>,
        bytes: u64,
        at: Vec<Resource>,
    }

    struct Record {
        entries: Vec<Entry>,
        peaks: Vec<Peak>,
        /// The Metal devices seen, for what they hold in all.
        devices: Vec<Handle<ProtocolObject<dyn MTLDevice>>>,
    }

    static RECORD: Mutex<Record> =
        Mutex::new(Record { entries: Vec::new(), peaks: Vec::new(), devices: Vec::new() });

    impl Record {
        /// Forget what Metal has freed.
        fn prune(&mut self) {
            self.entries.retain(|e| e.handle.0.load().is_some());
            self.devices.retain(|d| d.0.load().is_some());
        }

        fn live(&self, owner: Option<Owner>) -> impl Iterator<Item = &Resource> {
            self.entries
                .iter()
                .filter(move |e| owner.is_none_or(|o| e.owner == o))
                .map(|e| &e.resource)
        }

        fn peak(&mut self, owner: Option<Owner>) -> &mut Peak {
            let at = match self.peaks.iter().position(|p| p.owner == owner) {
                Some(at) => at,
                None => {
                    self.peaks.push(Peak { owner, bytes: 0, at: Vec::new() });
                    self.peaks.len() - 1
                }
            };
            &mut self.peaks[at]
        }

        /// `None` until a Metal resource is recorded, which on another backend
        /// is never.
        fn device(&self) -> Option<u64> {
            let devices = self.devices.iter().filter_map(|d| d.0.load());
            devices.map(|d| d.currentAllocatedSize() as u64).reduce(|a, b| a + b)
        }
    }

    pub(super) fn texture(
        texture: &wgpu::Texture,
    ) -> Option<Retained<ProtocolObject<dyn MTLResource>>> {
        let hal = unsafe { texture.as_hal::<Metal>() }?;
        Some(ProtocolObject::from_retained(hal.raw_handle().retain()))
    }

    pub(super) fn buffer(
        buffer: &wgpu::Buffer,
    ) -> Option<Retained<ProtocolObject<dyn MTLResource>>> {
        let hal = unsafe { buffer.as_hal::<Metal>() }?;
        Some(ProtocolObject::from_retained(hal.raw_handle().retain()))
    }

    fn owner(device: &wgpu::Device) -> Option<(Owner, Retained<ProtocolObject<dyn MTLDevice>>)> {
        let hal = unsafe { device.as_hal::<Metal>() }?;
        Some((&*hal as *const _ as Owner, hal.raw_device().clone()))
    }

    pub(super) fn record(
        device: &wgpu::Device,
        label: Option<&str>,
        shape: Shape,
        raw: Option<Retained<ProtocolObject<dyn MTLResource>>>,
    ) {
        let (Some(raw), Some((owner, mtl))) = (raw, owner(device)) else { return };
        let bytes = raw.allocatedSize() as u64;
        let resource = Resource { label: label.unwrap_or("(unlabelled)").to_owned(), shape, bytes };
        let mut record = RECORD.lock().unwrap_or_else(|e| e.into_inner());
        record.prune();
        // An owner with nothing alive may be a new device at a dead one's
        // address, which must not inherit its peak.
        if !record.entries.iter().any(|e| e.owner == owner) {
            record.peaks.retain(|p| p.owner != Some(owner));
        }
        record.entries.push(Entry { owner, resource, handle: Handle(Weak::from_retained(&raw)) });
        if !record
            .devices
            .iter()
            .any(|d| d.0.load().is_some_and(|d| Retained::as_ptr(&d) == Retained::as_ptr(&mtl)))
        {
            record.devices.push(Handle(Weak::from_retained(&mtl)));
        }
        // Totals rise only here, so re-counting here makes the peaks exact.
        // One device's own peak is a test's question only; kept for every
        // editor window the plugin ever opened, it would only accumulate.
        let keys: &[Option<Owner>] =
            if cfg!(any(test, feature = "test-support")) { &[Some(owner), None] } else { &[None] };
        for &key in keys {
            let live: u64 = record.live(key).map(|r| r.bytes).sum();
            if live > record.peak(key).bytes {
                let at = record.live(key).cloned().collect();
                *record.peak(key) = Peak { owner: key, bytes: live, at };
            }
        }
    }

    pub(super) fn snapshot(device: Option<&wgpu::Device>) -> GpuMemory {
        let owner = match device {
            Some(device) => match owner(device) {
                Some((owner, _)) => Some(owner),
                None => return GpuMemory::default(),
            },
            None => None,
        };
        let mut record = RECORD.lock().unwrap_or_else(|e| e.into_inner());
        record.prune();
        let mut resources: Vec<Resource> = record.live(owner).cloned().collect();
        resources.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.label.cmp(&b.label)));
        let live = resources.iter().map(|r| r.bytes).sum();
        let device = record.device();
        let peak = record.peak(owner);
        let totals = Totals { live, peak: peak.bytes, device };
        GpuMemory { resources, totals, at_peak: peak.at.clone() }
    }

    pub(super) fn totals() -> Totals {
        let mut record = RECORD.lock().unwrap_or_else(|e| e.into_inner());
        record.prune();
        let live = record.live(None).map(|r| r.bytes).sum();
        let device = record.device();
        Totals { live, peak: record.peak(None).bytes, device }
    }
}

#[cfg(not(target_vendor = "apple"))]
mod metal {
    use super::{GpuMemory, Shape, Totals};
    use crate::wgpu;

    pub(super) fn texture(_: &wgpu::Texture) -> Option<()> {
        None
    }

    pub(super) fn buffer(_: &wgpu::Buffer) -> Option<()> {
        None
    }

    pub(super) fn record(_: &wgpu::Device, _: Option<&str>, _: Shape, _: Option<()>) {}

    pub(super) fn snapshot(_: Option<&wgpu::Device>) -> GpuMemory {
        GpuMemory::default()
    }

    pub(super) fn totals() -> Totals {
        Totals::default()
    }
}

#[cfg(all(test, target_vendor = "apple"))]
mod tests {
    use super::*;

    fn target(device: &wgpu::Device, label: &str, side: u32) -> wgpu::Texture {
        texture(
            device,
            &wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: side, height: side, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
        )
    }

    /// A texture is on the record, at least its texels' worth, while Metal
    /// holds it — including while only a view does — and off it once freed,
    /// though the peak it made still names it.
    #[test]
    fn the_record_holds_what_metal_holds() {
        let Some((device, _queue)) = crate::test_gpu_device() else { return };
        let side = 512;
        let texels = u64::from(side * side) * 8;

        let texture = target(&device, "probe", side);
        let view = texture.create_view(&Default::default());
        let held = snapshot_of(&device);
        let probe: Vec<_> = held.resources.iter().filter(|r| r.label == "probe").collect();
        assert_eq!(probe.len(), 1, "{}", held.table());
        assert!(probe[0].bytes >= texels, "{} < {texels}", probe[0].bytes);
        assert!(held.totals.device.is_some_and(|d| d >= texels));

        drop(texture);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        assert!(
            snapshot_of(&device).resources.iter().any(|r| r.label == "probe"),
            "the view still holds it"
        );

        // A second target while the first is still held is the peak, and
        // the record keeps what made it.
        let second = target(&device, "second", side);
        drop(view);
        drop(second);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let freed = snapshot_of(&device);
        assert!(freed.resources.iter().all(|r| r.label != "probe"), "{}", freed.table());
        assert!(freed.totals.peak >= 2 * texels, "{}", freed.table());
        let at_peak: Vec<_> = freed.at_peak.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(at_peak, ["probe", "second"]);
        assert!(freed.table().contains("which allocating second set"), "{}", freed.table());
    }

    /// A buffer is on the record while Metal holds it and off it once freed:
    /// the path that reads the vendored `Buffer::raw_handle`.
    #[test]
    fn the_record_holds_a_buffer_while_metal_does() {
        let Some((device, queue)) = crate::test_gpu_device() else { return };
        let contents = vec![7u8; 65536];
        let held = buffer_init(
            &device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("probe_buffer"),
                contents: &contents,
                usage: wgpu::BufferUsages::STORAGE,
            },
        );
        let record = snapshot_of(&device);
        let probe: Vec<_> = record.resources.iter().filter(|r| r.label == "probe_buffer").collect();
        assert_eq!(probe.len(), 1, "{}", record.table());
        assert!(probe[0].bytes >= contents.len() as u64, "{}", record.table());

        // Its contents reach a GPU-private buffer by a copy wgpu queues and
        // flushes at the next submit, and until then that copy holds it.
        drop(held);
        queue.submit([]);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let freed = snapshot_of(&device);
        assert!(freed.resources.iter().all(|r| r.label != "probe_buffer"), "{}", freed.table());
    }
}
