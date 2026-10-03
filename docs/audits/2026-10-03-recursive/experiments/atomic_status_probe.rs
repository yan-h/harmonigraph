use crossbeam_utils::atomic::AtomicCell;
use nice_plug_core::plugin::ProcessStatus;
fn main() {
    println!("ProcessStatus: size={} align={} AtomicCell lock_free={}", std::mem::size_of::<ProcessStatus>(), std::mem::align_of::<ProcessStatus>(), AtomicCell::<ProcessStatus>::is_lock_free());
    println!("u32: size={} align={} AtomicCell lock_free={}; AtomicU32 size={}", std::mem::size_of::<u32>(), std::mem::align_of::<u32>(), AtomicCell::<u32>::is_lock_free(), std::mem::size_of::<std::sync::atomic::AtomicU32>());
    assert!(!AtomicCell::<ProcessStatus>::is_lock_free());
    assert!(AtomicCell::<u32>::is_lock_free());
}
