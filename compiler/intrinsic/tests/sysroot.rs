use intrinsic::sysroot::{PackageId, Sysroot};
use std::path::PathBuf;

#[test]
fn package_ids_cover_all_six_builtin_packages() {
    let expected = [
        (PackageId::BUILTIN, 0, "builtin"),
        (PackageId::META, 1, "meta"),
        (PackageId::BUILD, 2, "build"),
        (PackageId::ATTRS, 3, "attrs"),
        (PackageId::VERIFICATION, 4, "verification"),
        (PackageId::STD, 5, "std"),
    ];
    for (id, raw, name) in expected {
        assert_eq!(id.raw(), raw);
        assert_eq!(id.index(), raw as usize);
        assert!(id.is_sysroot());
        assert_eq!(id.to_string(), name);
    }
    assert_eq!(PackageId::USER_START.raw(), 6);
    assert!(!PackageId::USER_START.is_sysroot());
    assert_eq!(PackageId::USER_START.to_string(), "pkg#6");
}

#[test]
fn sysroot_discovers_all_six_builtin_packages_and_rejects_incomplete_dirs() {
    let library_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let sysroot = Sysroot::from_library_dir(&library_dir).expect("workspace library sysroot");
    let names: Vec<_> = sysroot.packages().map(|p| p.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["builtin", "meta", "build", "attrs", "verification", "std"]
    );
    for pkg in sysroot.packages() {
        assert_eq!(sysroot.package(pkg.package_id).unwrap().name, pkg.name);
        assert_eq!(
            sysroot.package_by_name(&pkg.name).unwrap().package_id,
            pkg.package_id
        );
        assert!(pkg.source_root.is_dir());
    }
    assert!(sysroot.package(PackageId::USER_START).is_none());
    assert!(sysroot.package_by_name("unknown").is_none());

    // Missing any of the six required packages rejects the directory.
    let temp = std::env::temp_dir().join(format!("luna-sysroot-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).unwrap();
    for present in ["builtin", "meta", "build", "attrs", "std"] {
        std::fs::create_dir_all(temp.join(present)).unwrap();
    }
    assert!(Sysroot::from_library_dir(&temp).is_none());
    let _ = std::fs::remove_dir_all(&temp);
}
