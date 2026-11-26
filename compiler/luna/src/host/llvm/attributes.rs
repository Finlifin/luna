//! LLVM 23 attribute emission, deduplication, and final machine-IR soundness guards.
use iir::{BehaviorAttributes, MemoryEffect, PointerParamAttrs};
use std::collections::BTreeMap;
use std::fmt;

/// Structured machine-IR side-effect flags recorded directly during LLVM emission.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct EmittedBodyEffects {
    pub has_trap_or_unwind: bool,
    pub has_volatile: bool,
    pub has_external_or_indirect_call: bool,
}

impl EmittedBodyEffects {
    pub(super) fn merge(&mut self, other: Self) {
        self.has_trap_or_unwind |= other.has_trap_or_unwind;
        self.has_volatile |= other.has_volatile;
        self.has_external_or_indirect_call |= other.has_external_or_indirect_call;
    }
}

impl fmt::Display for EmittedBodyEffects {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(emitted-body-effects (trap-or-unwind {}) (volatile {}) (external-or-indirect-call {}))",
            self.has_trap_or_unwind, self.has_volatile, self.has_external_or_indirect_call
        )
    }
}

/// Deduplicates `attributes #N = { ... }` groups for deterministic LLVM IR emission.
#[derive(Default)]
pub(super) struct AttributeTable {
    groups: Vec<String>,
    index: BTreeMap<String, usize>,
}

impl AttributeTable {
    fn intern_rendered(&mut self, rendered: String) -> Option<usize> {
        if rendered.is_empty() {
            return None;
        }
        if let Some(&id) = self.index.get(&rendered) {
            return Some(id);
        }
        let id = self.groups.len();
        self.groups.push(rendered.clone());
        self.index.insert(rendered, id);
        Some(id)
    }

    pub(super) fn intern(&mut self, attrs: &BehaviorAttributes) -> Option<usize> {
        self.intern_rendered(format_tokens_inner(attrs, true))
    }

    /// Call sites in LLVM IR may carry `memory(...)`, `nounwind`, `willreturn`,
    /// `nosync`, and `nofree`, but never `speculatable` (which LLVM restricts to
    /// function definitions and declarations).
    pub(super) fn intern_callsite(&mut self, attrs: &BehaviorAttributes) -> Option<usize> {
        self.intern_rendered(format_tokens_inner(attrs, false))
    }

    pub(super) fn render_declarations(&self) -> String {
        let mut out = String::new();
        for (id, body) in self.groups.iter().enumerate() {
            out.push_str(&format!("attributes #{id} = {{ {body} }}\n"));
        }
        out
    }
}

/// Apply final machine-IR soundness checks using structured side-effect flags
/// recorded directly during LLVM emission.
/// Reject speculation across machine traps and volatile memory accesses.
pub(super) fn sanitize_for_emitted_body(
    attrs: &mut BehaviorAttributes,
    effects: EmittedBodyEffects,
) {
    if effects.has_trap_or_unwind || effects.has_external_or_indirect_call {
        attrs.degrade_for_runtime_failure_access();
    }
    if effects.has_external_or_indirect_call {
        attrs.terminates = false;
        attrs.no_free = false;
        attrs.no_sync = false;
    }
    if effects.has_volatile {
        attrs.degrade_for_volatile_access();
    }
}

/// Format LLVM 23 function attribute tokens enforcing all P0-P5 soundness red lines.
#[cfg(test)]
pub(super) fn format_tokens(attrs: &BehaviorAttributes) -> String {
    format_tokens_inner(attrs, true)
}

fn format_tokens_inner(attrs: &BehaviorAttributes, allow_speculatable: bool) -> String {
    let mut tokens: Vec<&'static str> = Vec::new();
    if attrs.no_trap_or_unwind {
        tokens.push("nounwind");
    }
    if attrs.terminates && attrs.no_trap_or_unwind {
        tokens.push("willreturn");
    }
    if attrs.no_sync {
        tokens.push("nosync");
    }
    if attrs.no_free {
        tokens.push("nofree");
    }
    // Only certified machine behavior can advertise restricted memory access.
    if attrs.no_trap_or_unwind {
        match attrs.memory {
            MemoryEffect::None => tokens.push("memory(none)"),
            MemoryEffect::ArgMemRead => tokens.push("memory(argmem: read)"),
            MemoryEffect::ArgMemReadWrite => tokens.push("memory(argmem: readwrite)"),
            MemoryEffect::ReadOnly => tokens.push("memory(read)"),
            MemoryEffect::Unknown => {}
        }
    }
    // Red Line 1 & 2: `speculatable` requires total, trap-free, pure, sync/free-free
    // execution, and is only legal on functions/declarations (never call sites).
    if allow_speculatable
        && attrs.no_trap_or_unwind
        && attrs.terminates
        && attrs.no_free
        && attrs.no_sync
        && attrs.memory == MemoryEffect::None
    {
        tokens.push("speculatable");
    }
    tokens.join(" ")
}

/// Format LLVM 23 pointer parameter attributes (`ptr [readonly] [noalias] [captures(none)] [nonnull]`).
pub(super) fn format_pointer_param(param: Option<&PointerParamAttrs>) -> String {
    let mut out = String::from("ptr");
    if let Some(p) = param {
        if p.readonly {
            out.push_str(" readonly");
        }
        if p.noalias {
            out.push_str(" noalias");
        }
        if p.captures_none {
            out.push_str(" captures(none)");
        }
        if p.nonnull {
            out.push_str(" nonnull");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enforces_red_lines_in_llvm_token_formatting() {
        let p5 = BehaviorAttributes {
            memory: MemoryEffect::None,
            terminates: true,
            no_trap_or_unwind: true,
            no_free: true,
            no_sync: true,
            pointer_params: vec![],
        };
        assert_eq!(
            format_tokens(&p5),
            "nounwind willreturn nosync nofree memory(none) speculatable"
        );

        let mut table = AttributeTable::default();
        let fn_group = table.intern(&p5).unwrap();
        let cs_group = table.intern_callsite(&p5).unwrap();
        assert_ne!(fn_group, cs_group);
        let decls = table.render_declarations();
        assert!(decls.contains(&format!(
            "attributes #{cs_group} = {{ nounwind willreturn nosync nofree memory(none) }}\n"
        )));

        let p4 = BehaviorAttributes {
            terminates: false,
            ..p5.clone()
        };
        assert_eq!(format_tokens(&p4), "nounwind nosync nofree memory(none)");

        let mut trapped = p5.clone();
        sanitize_for_emitted_body(
            &mut trapped,
            EmittedBodyEffects {
                has_trap_or_unwind: true,
                has_volatile: false,
                has_external_or_indirect_call: false,
            },
        );
        let rendered = format_tokens(&trapped);
        assert!(!rendered.contains("memory(none)"));
        assert!(!rendered.contains("speculatable"));
        assert!(!rendered.contains("willreturn"));
        assert!(!rendered.contains("nounwind"));

        let mut volatile = p5.clone();
        sanitize_for_emitted_body(
            &mut volatile,
            EmittedBodyEffects {
                has_trap_or_unwind: false,
                has_volatile: true,
                has_external_or_indirect_call: false,
            },
        );
        let rendered_vol = format_tokens(&volatile);
        assert!(rendered_vol.contains("nounwind"));
        assert!(!rendered_vol.contains("nosync"));
        assert!(!rendered_vol.contains("memory(none)"));
        assert!(!rendered_vol.contains("speculatable"));

        let mut external = p5.clone();
        sanitize_for_emitted_body(
            &mut external,
            EmittedBodyEffects {
                has_trap_or_unwind: false,
                has_volatile: false,
                has_external_or_indirect_call: true,
            },
        );
        let rendered_ext = format_tokens(&external);
        assert!(!rendered_ext.contains("memory(none)"));
        assert!(!rendered_ext.contains("speculatable"));
        assert!(!rendered_ext.contains("nounwind"));
        assert!(!rendered_ext.contains("willreturn"));
        assert!(!rendered_ext.contains("nosync"));
        assert!(!rendered_ext.contains("nofree"));

        let mut clean = p5.clone();
        sanitize_for_emitted_body(&mut clean, EmittedBodyEffects::default());
        assert_eq!(clean, p5);
        assert_eq!(
            EmittedBodyEffects {
                has_trap_or_unwind: true,
                has_volatile: false,
                has_external_or_indirect_call: false,
            }
            .to_string(),
            "(emitted-body-effects (trap-or-unwind true) (volatile false) (external-or-indirect-call false))"
        );
    }
}
