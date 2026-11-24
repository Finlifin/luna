//! Validate merged `__flurry_macro_{kind,variadic,grammar}` attributes on macro handlers.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MacroKindTag {
    Ast,
    Peg,
    Tokens,
    Source,
}

impl std::fmt::Display for MacroKindTag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Ast => "ast",
            Self::Peg => "peg",
            Self::Tokens => "tokens",
            Self::Source => "source",
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MacroHandlerAttr {
    pub owner: OwnerId,
    pub kind: MacroKindTag,
    pub source: n::SourceRange,
}

impl std::fmt::Display for MacroHandlerAttr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(macro-handler-attribute (owner {}) (kind {}) (source {} {}))",
            self.owner, self.kind, self.source.start, self.source.end
        )
    }
}

/// Validate `__flurry_macro_*` keys on comptime macro-handler function definitions.
pub fn macro_handlers(
    package: &hir::Package<'_>,
    merged: &[MergedTarget<'_>],
    budget: &mut comptime_runtime::Budget,
) -> Result<Vec<MacroHandlerAttr>, MergeFailure> {
    let mut output = Vec::new();
    for target in merged {
        let location = target.source();
        budget.consume(1).map_err(|e| MergeFailure {
            source: location,
            message: e.to_string(),
        })?;
        let kind_entry = target.get("__flurry_macro_kind");
        let variadic_entry = target.get("__flurry_macro_variadic");
        let grammar_entry = target.get("__flurry_macro_grammar");
        let Some(first) = kind_entry.or(variadic_entry).or(grammar_entry) else {
            continue;
        };
        let location = first.origin.source;
        let failure = |source: n::SourceRange, message: &str| MergeFailure {
            source,
            message: message.into(),
        };
        for present in [kind_entry, variadic_entry, grammar_entry]
            .into_iter()
            .flatten()
        {
            if present.entry.is_none() || present.origin.configuration_key.is_some() {
                return Err(failure(
                    present.origin.source,
                    "__flurry_macro_* attributes cannot be attached via definition configuration; use ^attrs.macros.*",
                ));
            }
        }
        let hir::AttributeTarget::Definition(owner) = target.target else {
            return Err(failure(
                location,
                "__flurry_macro_* requires a `comptime fn` macro handler definition",
            ));
        };
        let comptime_callable = package.item(owner).is_some_and(|item| {
            matches!(
                &item.kind,
                ItemKind::Fn(sig, _)
                    if sig.modifiers.is_comptime
                        && !sig.modifiers.is_quote
                        && !sig.modifiers.is_normal_form_head
            )
        });
        if !comptime_callable {
            return Err(failure(
                location,
                "__flurry_macro_* requires a `comptime fn` macro handler definition",
            ));
        }
        let Some(kind_selected) = kind_entry else {
            return Err(failure(
                location,
                "__flurry_macro_variadic and __flurry_macro_grammar require __flurry_macro_kind",
            ));
        };
        let kind_source = kind_selected.origin.source;
        if kind_selected.value.ty != p(P::Symbol) {
            return Err(failure(
                kind_source,
                "__flurry_macro_kind requires the Symbol domain",
            ));
        }
        let kind_name = match &kind_selected.value.kind {
            K::Literal(n::Literal::Symbol(name)) => name.as_str(),
            K::ComptimeValue(Term::Symbol(name)) => name.as_ref(),
            _ => {
                return Err(failure(
                    kind_source,
                    "__flurry_macro_kind requires a completed Symbol value",
                ));
            }
        };
        let kind = match kind_name {
            "ast" => MacroKindTag::Ast,
            "peg" => MacroKindTag::Peg,
            "tokens" => MacroKindTag::Tokens,
            "source" => MacroKindTag::Source,
            _ => {
                return Err(failure(
                    kind_source,
                    "unsupported __flurry_macro_kind; expected .ast, .peg, .tokens, or .source",
                ));
            }
        };
        if let Some(variadic) = variadic_entry {
            let var_source = variadic.origin.source;
            if variadic.value.ty != p(P::Bool) {
                return Err(failure(
                    var_source,
                    "__flurry_macro_variadic requires the Bool domain",
                ));
            }
            if !matches!(
                variadic.value.kind,
                K::Literal(n::Literal::Bool(_)) | K::ComptimeValue(Term::Bool(_))
            ) {
                return Err(failure(
                    var_source,
                    "__flurry_macro_variadic requires a completed Bool value",
                ));
            }
            if kind != MacroKindTag::Ast {
                return Err(failure(
                    var_source,
                    "__flurry_macro_variadic is only valid for .ast macros",
                ));
            }
        }
        if let Some(grammar) = grammar_entry {
            let gram_source = grammar.origin.source;
            if grammar.value.ty != p(P::Str) {
                return Err(failure(
                    gram_source,
                    "__flurry_macro_grammar requires the Str domain",
                ));
            }
            let grammar_text = match &grammar.value.kind {
                K::Literal(n::Literal::String(text)) => text.as_str(),
                K::ComptimeValue(Term::String(text)) => text.as_ref(),
                _ => {
                    return Err(failure(
                        gram_source,
                        "__flurry_macro_grammar requires a completed str value",
                    ));
                }
            };
            if kind != MacroKindTag::Peg {
                return Err(failure(
                    gram_source,
                    "__flurry_macro_grammar is only valid for .peg macros",
                ));
            }
            if grammar_text.trim().is_empty() {
                return Err(failure(
                    gram_source,
                    "__flurry_macro_grammar requires a non-empty grammar string",
                ));
            }
        } else if kind == MacroKindTag::Peg {
            return Err(failure(
                kind_source,
                "__flurry_macro_kind .peg requires __flurry_macro_grammar",
            ));
        }
        output.push(MacroHandlerAttr {
            owner,
            kind,
            source: kind_source,
        });
    }
    Ok(output)
}
