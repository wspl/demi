use demi_runner_host::volumes::growth_wanted;

/// A managed volume asks to double once its free space falls below its
/// reserve: a tenth of it, but at least a quarter of it up to 256 MiB.
#[test]
fn growth_reserve_has_fraction_floor_and_small_volume_cap() {
    let mib = 1024 * 1024;
    assert_eq!(growth_wanted(128 * mib, 32 * mib).unwrap(), None);
    assert_eq!(growth_wanted(128 * mib, 31 * mib).unwrap(), Some(256 * mib));
    assert_eq!(
        growth_wanted(2048 * mib, 255 * mib).unwrap(),
        Some(4096 * mib)
    );
    assert_eq!(growth_wanted(8192 * mib, 820 * mib).unwrap(), None);
    assert_eq!(
        growth_wanted(8192 * mib, 819 * mib).unwrap(),
        Some(16384 * mib)
    );
    assert!(growth_wanted(u64::MAX, 0).is_err());
}
