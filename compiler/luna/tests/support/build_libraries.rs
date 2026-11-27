//! Actual library dependencies for targetless build-context and cache fixtures.
use luna::packages::SourcePackage;

const SUPPORTING_LIBRARIES: [(u32, u64, &str); 3] =
    [(11, 400, "builtin"), (12, 500, "meta"), (13, 600, "attrs")];

pub(crate) fn supporting_libraries(builtin: &std::path::Path) -> Vec<SourcePackage> {
    let root = builtin
        .parent()
        .and_then(|dir| dir.parent())
        .expect("builtin entry root");
    SUPPORTING_LIBRARIES
        .into_iter()
        .map(|(resolution_id, identity, name)| SourcePackage {
            resolution_id,
            identity,
            name: name.into(),
            entry: match name {
                "builtin" => builtin.to_path_buf(),
                "meta" => meta_entry(root),
                _ => std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join(format!("../../library/{name}/lib.fl")),
            },
        })
        .collect()
}

fn copy_dir_recursive(source: &std::path::Path, destination: &std::path::Path) {
    std::fs::create_dir_all(destination).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let file_type = entry.file_type().unwrap();
        let target = destination.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&entry.path(), &target);
        } else if file_type.is_file() {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn meta_entry(root: &std::path::Path) -> std::path::PathBuf {
    // Targetless `--emit nir` compiles without Copy/Drop language-item selectors,
    // making `SourceSpan` a move type under P4 verification; adapt `SourceSpanOps`
    // in the test fixture copy just as `support/meta_source.rs` does.
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library/meta");
    let destination = root.join("meta");
    copy_dir_recursive(&source, &destination);
    let span_targetless =
        include_str!("../../../../library/meta/ast_impl/span.fl").replace("pure fn ", "fn ");
    std::fs::write(destination.join("ast_impl/span.fl"), span_targetless).unwrap();
    destination.join("lib.fl")
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
    nir::intrinsics::Operation::scalar_math()
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
    // the shipped targetless builtin dependency closure required by real
    // library/{meta,attrs,build}; full sysroot target tests separately cover
    // runtime usize/intrinsic modules.
    let directory = root.join("builtin");
    for subdir in [
        "numeric",
        "types",
        "text_impl",
        "protocols",
        "protocols/capabilities",
        "protocols/formatting",
    ] {
        std::fs::create_dir_all(directory.join(subdir)).unwrap();
    }
    let write = |rel: &str, text: &str| {
        std::fs::write(directory.join(rel), text).unwrap();
    };
    write(
        "primitives.fl",
        include_str!("../../../../library/builtin/primitives.fl"),
    );
    write(
        "bindings.fl",
        include_str!("../../../../library/builtin/bindings.fl"),
    );
    write(
        "implicit_bindings.fl",
        include_str!("../../../../library/builtin/implicit_bindings.fl"),
    );
    write(
        "numeric/comparison.fl",
        include_str!("../../../../library/builtin/numeric/comparison.fl"),
    );
    write(
        "types/option.fl",
        include_str!("../../../../library/builtin/types/option.fl"),
    );
    write(
        "types/result.fl",
        include_str!("../../../../library/builtin/types/result.fl"),
    );
    write(
        "types/tuple.fl",
        include_str!("../../../../library/builtin/types/tuple.fl"),
    );
    let bytes_targetless =
        include_str!("../../../../library/builtin/text_impl/bytes.fl").replace("pure fn ", "fn ");
    write("text_impl/bytes.fl", &bytes_targetless);
    write(
        "protocols/resources.fl",
        include_str!("../../../../library/builtin/protocols/resources.fl"),
    );
    write(
        "protocols/ops.fl",
        include_str!("../../../../library/builtin/protocols/ops.fl"),
    );
    write(
        "protocols/staging.fl",
        include_str!("../../../../library/builtin/protocols/staging.fl"),
    );
    write(
        "protocols/implicit_passing.fl",
        include_str!("../../../../library/builtin/protocols/implicit_passing.fl"),
    );
    write(
        "protocols/patterns.fl",
        include_str!("../../../../library/builtin/protocols/patterns.fl"),
    );
    write(
        "protocols/conversion.fl",
        include_str!("../../../../library/builtin/protocols/conversion.fl"),
    );
    write(
        "protocols/capabilities/effects.fl",
        include_str!("../../../../library/builtin/protocols/capabilities/effects.fl"),
    );
    write(
        "protocols/capabilities/errors.fl",
        include_str!("../../../../library/builtin/protocols/capabilities/errors.fl"),
    );
    let traits_full = include_str!("../../../../library/builtin/protocols/traits.fl");
    let traits_targetless = traits_full
        .replace("pub use hash.{BuildHasher, Hash, Hasher}\n", "")
        .replace(
            "impl Default for usize {\n    fn default() -> Self {\n        0\n    }\n}\n",
            "",
        )
        .replace(
            "impl Default for isize {\n    fn default() -> Self {\n        0\n    }\n}\n",
            "",
        );
    write("protocols/traits.fl", &traits_targetless);
    let options_full = include_str!("../../../../library/builtin/protocols/formatting/options.fl");
    let (options_before_pad_counts, options_after_pad_counts) = options_full
        .split_once("pub pure fn compute_pad_counts(")
        .expect("options.fl compute_pad_counts marker");
    let (_, options_from_comptime) = options_after_pad_counts
        .split_once("{- ── Comptime property scanner & parser")
        .expect("options.fl comptime scanner marker");
    let options_comptime = options_from_comptime
        .split("{- ── Runtime UTF-8 and padding helpers")
        .next()
        .expect("options.fl runtime formatting marker");
    let options_targetless = format!(
        "{options_before_pad_counts}{{- ── Comptime property scanner & parser{options_comptime}"
    );
    write("protocols/formatting/options.fl", &options_targetless);
    write(
        "protocols/formatting/writer.fl",
        concat!(
            "use bindings.{str_byte, str_byte_length}\n",
            "use primitives.{str, u64, u8, void}\n",
            "pub trait ByteWriter {\n",
            "    assoc Err: Type;\n",
            "    fn write_byte(*self, byte: u8) -> !Err void;\n",
            "    fn write_str(*self, text: str) -> !Err void {\n",
            "        let length = str_byte_length(text);\n",
            "        var index: u64 = 0;\n",
            "        while index < length {\n",
            "            let maybe_byte = str_byte(text, index);\n",
            "            let valid = maybe_byte match { _? => true, null => false };\n",
            "            assumes valid;\n",
            "            let byte: u8 = maybe_byte match { found? => found, null => 0 };\n",
            "            self.write_byte(byte)!;\n",
            "            index = index + 1;\n",
            "        }\n",
            "    }\n",
            "}\n",
        ),
    );
    write(
        "protocols/formatting/debug.fl",
        concat!(
            "use primitives.void;\n",
            "use protocols.formatting.writer.ByteWriter;\n",
            "pub trait Debug {\n",
            "    fn write_debug(*self, output: *W) -> !W.as(ByteWriter).Err void where W :- ByteWriter;\n",
            "}\n",
        ),
    );
    let range_full = include_str!("../../../../library/builtin/range.fl");
    let range_head = range_full
        .split("-- ── Integer Iterator implementations")
        .next()
        .expect("range.fl integer iterator marker")
        .replace("use mem_impl.raw as raw_mem;\n", "")
        .replace("use protocols.iterator.{Iterator, emit_item}\n", "");
    let (range_before_bound_impls, range_after_bound_impls) = range_head
        .split_once("-- Clone-only enum payloads await borrowed pointer-match views.")
        .expect("range.fl bound impl marker");
    let (_, range_from_full) = range_after_bound_impls
        .split_once("{- An unbounded range (`..`). -}")
        .expect("range.fl RangeFull marker");
    let range_targetless =
        format!("{range_before_bound_impls}{{- An unbounded range (`..`). -}}{range_from_full}");
    write("range.fl", &range_targetless);

    let entry = directory.join("lib.fl");
    std::fs::write(
        &entry,
        concat!(
            "use numeric.comparison;\n",
            "use protocols.{\n",
            "    conversion,\n",
            "    implicit_passing,\n",
            "    ops,\n",
            "    patterns,\n",
            "    resources,\n",
            "    staging as staging_impl,\n",
            "    traits,\n",
            "};\n",
            "use protocols.capabilities.{effects as capability_effects, errors as capability_errors};\n",
            "use protocols.formatting.{debug, options as formatting_options, writer};\n",
            "use range;\n",
            "use text_impl.bytes;\n",
            "use types.{option, result, tuple};\n",
            "\n",
            "pub use capability_effects.{HandleCapturingEffect, HandleEffect};\n",
            "pub use capability_errors.HandleError;\n",
            "pub use implicit_passing.{CallDescription, Context, ImplicitPassing};\n",
            "pub use ops.*;\n",
            "pub use option.OptionalOps;\n",
            "pub use patterns.*;\n",
            "pub use primitives.*;\n",
            "pub use range.{\n",
            "    Bound, Range, RangeBounds, RangeFrom, RangeFull, RangeInclusive, RangeTo, RangeToInclusive,\n",
            "};\n",
            "pub use result.ErrorResultOps;\n",
            "pub use staging_impl.{Splice, splice_auto};\n",
            "pub use bytes.TextBytes;\n",
            "pub use traits.*;\n",
            "pub use tuple.Tuple;\n",
            "\n",
            "pub mod compiler {\n",
            "    pub use bindings.*;\n",
            "}\n",
            "\n",
            "pub mod convert {\n",
            "    pub use conversion.{\n",
            "        AsMut, AsRef, ConversionErr, From, InfallibleErr, Into, LiteralInto, TryFrom, TryInto,\n",
            "    };\n",
            "}\n",
            "\n",
            "pub mod capabilities {\n",
            "    pub use capability_effects.{HandleCapturingEffect, HandleEffect};\n",
            "    pub use capability_errors.HandleError;\n",
            "}\n",
            "\n",
            "pub mod fmt {\n",
            "    pub use debug.Debug;\n",
            "    pub use formatting_options.FormatOptions;\n",
            "    pub use writer.ByteWriter;\n",
            "\n",
            "    pub mod options {\n",
            "        pub use formatting_options.*;\n",
            "    }\n",
            "}\n",
            "\n",
            "pub mod math {\n",
            "    pub use comparison.{Eq, Ord, Order, TotalFloat, TotalOrd};\n",
            "}\n",
            "\n",
            "pub mod staging {\n",
            "    pub use protocols.staging.*;\n",
            "}\n",
            "\n",
            "pub mod prelude {\n",
            "    pub use .math;\n",
            "    pub use capability_effects.{HandleCapturingEffect, HandleEffect};\n",
            "    pub use capability_errors.HandleError;\n",
            "    pub use implicit_passing.{CallDescription, Context, ImplicitPassing};\n",
            "    pub use debug.Debug;\n",
            "    pub use ops.*;\n",
            "    pub use option.OptionalOps;\n",
            "    pub use range.{\n",
            "        Bound, Range, RangeBounds, RangeFrom, RangeFull, RangeInclusive, RangeTo, RangeToInclusive,\n",
            "    };\n",
            "    pub use patterns.*;\n",
            "    pub use primitives.*;\n",
            "    pub use protocols.staging.{Splice, splice_auto};\n",
            "    pub use result.ErrorResultOps;\n",
            "    pub use text_impl.bytes.TextBytes;\n",
            "    pub use traits.*;\n",
            "    pub use tuple.Tuple;\n",
            "}\n",
        ),
    )
    .unwrap();
    entry
}
