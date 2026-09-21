use super::*;

#[derive(Default)]
struct Host {
    active: AtomicBool,
    invalid_rescan: AtomicBool,
    restarts: AtomicU32,
    rescans: AtomicU32,
}
unsafe fn host_data<'a>(host: *const clap_host) -> &'a Host {
    &*((*host).host_data.cast())
}
unsafe extern "C" fn rescan(host: *const clap_host, flags: u32) {
    let host = host_data(host);
    if flags & CLAP_PARAM_RESCAN_ALL != 0 {
        if host.active.load(SeqCst) {
            host.invalid_rescan.store(true, SeqCst);
        }
        host.rescans.fetch_add(1, SeqCst);
    }
}
unsafe extern "C" fn restart(host: *const clap_host) {
    host_data(host).restarts.fetch_add(1, SeqCst);
}
unsafe extern "C" fn callback(_: *const clap_host) {}
static HOST_PARAMS: clap_host_params =
    clap_host_params { rescan: Some(rescan), clear: None, request_flush: None };
unsafe extern "C" fn extension(_: *const clap_host, id: *const c_char) -> *const c_void {
    if CStr::from_ptr(id) == CLAP_EXT_PARAMS {
        (&HOST_PARAMS as *const clap_host_params).cast()
    } else {
        ptr::null()
    }
}
fn host(state: &Host) -> clap_host {
    clap_host {
        clap_version: CLAP_VERSION,
        host_data: (state as *const Host).cast_mut().cast(),
        name: c"Lifecycle fixture".as_ptr(),
        vendor: c"Harmonigraph".as_ptr(),
        url: c"".as_ptr(),
        version: c"1".as_ptr(),
        get_extension: Some(extension),
        request_restart: Some(restart),
        request_process: None,
        request_callback: Some(callback),
    }
}

#[test]
fn expansion_rescans_after_deactivate_returns_and_preserves_signed_value() {
    unsafe {
        let state = Host::default();
        let host = host(&state);
        let plugin = create(&FACTORY, &host, DESCRIPTOR.id);
        assert!(!plugin.is_null());
        assert!(init(plugin));
        assert!(activate(plugin, 44100., 1, 512));
        state.active.store(true, SeqCst);
        let p = probe(plugin);
        p.value.store(3f64.to_bits(), SeqCst);
        p.requested.store(10, SeqCst);
        main_thread(plugin);
        assert_eq!(state.restarts.load(SeqCst), 1);
        assert_eq!(p.span.load(SeqCst), 5);
        deactivate(plugin);
        // The host remains active until the callback has returned.
        assert_eq!(state.rescans.load(SeqCst), 0);
        state.active.store(false, SeqCst);
        // Deliberately skip on_main: hosts can immediately reactivate.
        assert!(activate(plugin, 44100., 1, 512));
        state.active.store(true, SeqCst);
        assert_eq!(p.span.load(SeqCst), 10);
        assert_eq!(f64::from_bits(p.value.load(SeqCst)), 3.);
        assert_eq!(state.rescans.load(SeqCst), 1);
        assert!(!state.invalid_rescan.load(SeqCst));
        let mut metadata = std::mem::MaybeUninit::uninit();
        assert!(info(plugin, OFFSET, metadata.as_mut_ptr()));
        let metadata = metadata.assume_init();
        assert_eq!((metadata.min_value, metadata.max_value), (-10., 10.));
        deactivate(plugin);
        state.active.store(false, SeqCst);
        destroy(plugin);
    }
}

unsafe extern "C" fn read(stream: *const clap_istream, dest: *mut c_void, count: u64) -> i64 {
    let bytes = &mut *(*stream).ctx.cast::<&[u8]>();
    // Exercise hosts that return short reads, rather than assuming one call.
    let count = (count as usize).min(bytes.len()).min(3);
    ptr::copy_nonoverlapping(bytes.as_ptr(), dest.cast(), count);
    *bytes = &bytes[count..];
    count as i64
}
#[test]
fn active_restore_waits_for_inactivity_and_restores_range_before_readback() {
    unsafe {
        let state = Host::default();
        let host = host(&state);
        let plugin = create(&FACTORY, &host, DESCRIPTOR.id);
        assert!(init(plugin));
        assert!(activate(plugin, 44100., 1, 512));
        state.active.store(true, SeqCst);
        let bytes =
            [b"RNG1".as_slice(), &20u32.to_le_bytes(), &(-13f64).to_bits().to_le_bytes()].concat();
        let mut input = bytes.as_slice();
        let stream = clap_istream { ctx: (&mut input as *mut &[u8]).cast(), read: Some(read) };
        assert!(load(plugin, &stream));
        assert_eq!(probe(plugin).span.load(SeqCst), 5);
        assert_eq!(state.restarts.load(SeqCst), 1);
        deactivate(plugin);
        state.active.store(false, SeqCst);
        // Cover the other legal path, an inactive main-thread callback.
        main_thread(plugin);
        assert_eq!(probe(plugin).span.load(SeqCst), 20);
        let mut restored = 0.;
        assert!(value(plugin, OFFSET, &mut restored));
        assert_eq!(restored, -13.);
        assert_eq!(state.rescans.load(SeqCst), 1);
        assert!(!state.invalid_rescan.load(SeqCst));
        destroy(plugin);
    }
}
