//! Actual library dependencies for targetless build-context and cache fixtures.
use luna::packages::SourcePackage;

const SUPPORTING_LIBRARIES: [(u32, u64, &str); 3] =
    [(11, 400, "builtin"), (12, 500, "meta"), (13, 600, "attrs")];

pub(crate) fn supporting_libraries(builtin: &std::path::Path) -> Vec<SourcePackage> {
    SUPPORTING_LIBRARIES
        .into_iter()
        .map(|(resolution_id, identity, name)| SourcePackage {
            resolution_id,
            identity,
            name: name.into(),
            entry: if name == "builtin" {
                builtin.to_path_buf()
            } else {
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join(format!("../../library/{name}/lib.fl"))
            },
        })
        .collect()
}

pub(crate) fn supporting_dependencies(requesters: &[u32]) -> Vec<resolve::DependencyEdge> {
    requesters
        .iter()
        .flat_map(|&requester| {
            SUPPORTING_LIBRARIES
                .into_iter()
                .filter_map(move |(provider, _, alias)| {
                    (requester != provider).then_some(resolve::DependencyEdge {
                        requester,
                        alias: alias.into(),
                        provider,
                    })
                })
        })
        .collect()
}

pub(crate) fn comptime_intrinsics() -> Vec<(nir::intrinsics::Operation, String)> {
    nir::intrinsics::Operation::ALL
        .iter()
        .map(|op| {
            (
                *op,
                format!("meta.numeric.scalar_math.target.Scalar.{}", op.name()),
            )
        })
        .collect()
}

pub(crate) fn builtin_entry(root: &std::path::Path) -> std::path::PathBuf {
    // Build-origin checks also run without a code-generation target. Retain
    // the shipped binding/comparison dependency closure here; full sysroot
    // target and metadata tests separately cover the runtime library.
    let directory = root.join("builtin");
    std::fs::create_dir_all(&directory).unwrap();
    let entry = directory.join("lib.fl");
    std::fs::write(
        &entry,
        concat!(
            "mod primitives {\n",
            include_str!("../../../../library/builtin/primitives.fl"),
            "\n} pub use primitives.*;\n",
            include_str!("../../../../library/builtin/numeric/comparison.fl"),
            "\npub mod compiler {\n",
            include_str!("../../../../library/builtin/bindings.fl"),
            "\n}\n",
        ),
    )
    .unwrap();
    entry
}
