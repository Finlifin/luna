//! Deterministic, context-free S-expression formatting for HIR trees.
//!
//! Text (including identifiers and symbols) is always quoted. Backslash, quote,
//! newline, carriage return, and tab use escapes; other control characters use
//! `\uNNNN`. Optional fields explicitly distinguish `(none)` and `(some ...)`.
//! Existing source-like `Ident`, `Path`, and ID displays remain unchanged.

use std::fmt::{self, Display, Write};

use rustc_span::Span;
use symbol::{DefId, PathAnchor};

use crate::common::{Arg, FnParam, PathSegment, TyParam, TyParamKind};
use crate::expr::{ClosureParam, FnTypeModifiers, ObjectEntry};
use crate::item::{
    EffectGroupDef, EffectModifiers, FnModifiers, TestTemplateInterface, TestTemplatePort, UseKind,
    UsePath,
};
use crate::pattern::BoundType;
use crate::*;

macro_rules! form {
    ($f:expr, $tag:expr $(, $field:literal => $value:expr)* $(,)?) => {{
        write!($f, "({}", $tag)?;
        $(write!($f, " ({} {})", $field, $value)?;)*
        $f.write_str(")")
    }};
}

struct Quoted<'a>(&'a str);
impl Display for Quoted<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_char('"')?;
        for c in self.0.chars() {
            match c {
                '"' => f.write_str("\\\"")?,
                '\\' => f.write_str("\\\\")?,
                '\n' => f.write_str("\\n")?,
                '\r' => f.write_str("\\r")?,
                '\t' => f.write_str("\\t")?,
                c if c.is_control() => write!(f, "\\u{:04x}", c as u32)?,
                c => f.write_char(c)?,
            }
        }
        f.write_char('"')
    }
}

struct List<'a, T>(&'a [T]);
impl<T: Display> Display for List<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(")?;
        for (i, value) in self.0.iter().enumerate() {
            if i != 0 {
                f.write_str(" ")?;
            }
            write!(f, "{value}")?;
        }
        f.write_str(")")
    }
}

struct Maybe<T>(Option<T>);
impl<T: Display> Display for Maybe<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Some(value) => write!(f, "(some {value})"),
            None => f.write_str("(none)"),
        }
    }
}

struct Location(Span);
impl Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        form!(f, "span", "lo" => self.0.lo().0, "hi" => self.0.hi().0)
    }
}

struct Name(Ident);
impl Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        form!(f, "ident", "name" => Quoted(&self.0.name), "span" => Location(self.0.span))
    }
}

struct Id(HirId);
impl Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        form!(f, "hir-id", "owner" => self.0.owner.def_id.raw(), "local" => self.0.local_id.raw())
    }
}

struct Owner(OwnerId);
impl Display for Owner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(owner-id {})", self.0.def_id.raw())
    }
}

struct Definition(DefId);
impl Display for Definition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        form!(f, "def-id", "package" => self.0.pkg, "index" => self.0.index)
    }
}

impl Display for BodyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        form!(f, "body-id", "owner" => self.hir_id.owner.def_id.raw(), "local" => self.hir_id.local_id.raw())
    }
}

struct Owners<'a>(&'a [OwnerId]);
impl Display for Owners<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(")?;
        for (i, id) in self.0.iter().enumerate() {
            if i != 0 {
                f.write_str(" ")?;
            }
            write!(f, "{}", Owner(*id))?;
        }
        f.write_str(")")
    }
}

impl Display for Package<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(hir-package\n  (root {})\n  (owners",
            Owner(self.root_mod)
        )?;
        for (id, info) in self.owners() {
            write!(f, "\n    (owner (id {}) (node ", Owner(id))?;
            match info.node {
                OwnerNode::Item(item) => write!(f, "{item}")?,
            }
            f.write_str(") (node-index")?;
            // This is an index of the trees, not a second recursive expansion.
            for (local, entry) in info.nodes.nodes.iter_enumerated() {
                if let Some(entry) = entry {
                    write!(
                        f,
                        " (node-ref (id {}) (parent {}) (kind {}))",
                        Id(HirId::new(id, local)),
                        Id(HirId::new(id, entry.parent)),
                        Quoted(entry.node.kind_name())
                    )?;
                }
            }
            f.write_str("))")?;
        }
        f.write_str(")\n  (bodies")?;
        let mut bodies: Vec<_> = self.bodies().collect();
        bodies.sort_unstable_by_key(|(id, _)| {
            (id.hir_id.owner.def_id.raw(), id.hir_id.local_id.raw())
        });
        for (id, body) in bodies {
            write!(f, "\n    (body-entry (id {id}) (value {body}))")?;
        }
        f.write_str(") (definitions")?;
        let mut definitions: Vec<_> = self.definitions.iter().collect();
        definitions.sort_unstable_by_key(|(id, _)| (id.pkg, id.index));
        for (definition, owner) in definitions {
            write!(
                f,
                " (binding {} {})",
                Definition(*definition),
                Owner(*owner)
            )?;
        }
        f.write_str(") (member-projections")?;
        let mut projections: Vec<_> = self.member_projections.iter().collect();
        projections.sort_unstable_by_key(|(id, _)| (id.owner.def_id.raw(), id.local_id.raw()));
        for (expression, definition) in projections {
            write!(
                f,
                " (member (expression {}) (target {}))",
                Id(*expression),
                Definition(*definition)
            )?;
        }
        for (label, table) in [
            (
                "deferred-member-projections",
                &self.deferred_member_projections,
            ),
            ("member-access", &self.member_access),
        ] {
            write!(f, ") ({label}")?;
            let mut deferred: Vec<_> = table.iter().collect();
            deferred.sort_unstable_by_key(|(id, _)| (id.owner.def_id.raw(), id.local_id.raw()));
            for (expression, candidates) in deferred {
                write!(
                    f,
                    " (projection (expression {}) (candidates",
                    Id(*expression)
                )?;
                let mut candidates = candidates.to_vec();
                candidates.sort_unstable_by_key(|(container, member)| {
                    (container.pkg, container.index, member.pkg, member.index)
                });
                for (container, member) in candidates {
                    write!(
                        f,
                        " (member (container {}) (target {}))",
                        Definition(container),
                        Definition(member)
                    )?;
                }
                f.write_str("))")?;
            }
        }
        f.write_str(") (lexical-parents")?;
        let mut parents = self.lexical_parents.iter().collect::<Vec<_>>();
        parents.sort_unstable_by_key(|(child, _)| child.def_id.raw());
        for (child, parent) in parents {
            write!(
                f,
                " (lexical-parent (child (owner-id {})) (parent (owner-id {})))",
                child.def_id.raw(),
                parent.def_id.raw()
            )?;
        }
        f.write_str(") (staged-parameter-definitions")?;
        let mut staged = self.staged_parameter_definitions.iter().collect::<Vec<_>>();
        staged.sort_unstable_by_key(|(binding, _)| {
            (binding.owner.def_id.raw(), binding.local_id.raw())
        });
        for (binding, definition) in staged {
            write!(
                f,
                " (staged-input (binding {}) (definition {}))",
                Id(*binding),
                Definition(*definition)
            )?;
        }
        f.write_str(") (definition-attributes")?;
        for attribute in &self.definition_attributes {
            write!(f, " {attribute}")?;
        }
        f.write_str(") (pending-semantics")?;
        for (span, feature) in &self.pending_semantics {
            write!(f, " (pending {} {})", Location(*span), Quoted(feature))?;
        }
        f.write_str("))")
    }
}

impl Display for ObjectEntry<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Value(value) => form!(f, "object-value", "value" => value),
            Self::Property(field) => form!(f, "object-property", "field" => field),
        }
    }
}

impl Display for AttributeTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Definition(owner) => Owner(*owner).fmt(f),
            Self::Member(id) => Id(*id).fmt(f),
        }
    }
}

impl Display for DeferredDefinitionAttribute<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(key) = &self.configuration_key {
            return form!(f, "deferred-configuration", "target" => self.target,
                "lexical-owner" => Owner(self.lexical_owner), "key" => Quoted(key),
                "expression" => Quoted(&self.expression), "hir-expression" => self.value, "span" => Location(self.span));
        }
        form!(f, "deferred-attribute", "target" => self.target,
            "lexical-owner" => Owner(self.lexical_owner),
            "expression" => Quoted(&self.expression), "hir-expression" => self.value, "span" => Location(self.span))
    }
}

impl Display for Item<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        form!(f, "item", "id" => Id(self.hir_id()), "name" => Name(self.ident), "kind" => self.kind, "span" => Location(self.span))
    }
}

impl Display for ItemKind<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fn(sig, body) => form!(f, "fn", "signature" => sig, "body" => body),
            Self::FunctionTemplateInstance {
                template,
                signature,
                explicit_parameters,
                parameters,
                modifiers,
                body,
            } => {
                form!(f, "function-template-instance", "template" => template,
                    "signature" => Maybe(signature.as_ref()),
                    "explicit-parameters" => Maybe(explicit_parameters.map(List)),
                    "parameters" => List(parameters), "modifiers" => modifiers, "body" => body)
            }
            Self::TestTemplateInstance {
                interface,
                template,
                ports,
                body,
            } => {
                form!(f, "test-template-instance", "template" => template, "interface" => Maybe(interface.as_ref()), "ports" => List(ports), "body" => body)
            }
            Self::Struct(def) => form!(f, "struct", "definition" => def),
            Self::Enum(def) => form!(f, "enum", "definition" => def),
            Self::Mod(def) => form!(f, "mod", "items" => Owners(&def.items)),
            Self::Impl(def) => form!(f, "impl", "definition" => def),
            Self::Trait(def) => form!(f, "trait", "definition" => def),
            Self::Effect(sig, modifiers) => form!(f, "effect-operation", "signature" => sig,
                "modifiers" => modifiers),
            Self::EffectGroup(group) => group.fmt(f),
            Self::FnDeclaration(sig) => form!(f, "fn-declaration", "signature" => sig),
            Self::Associated(decl) => form!(f, "associated", "params" => List(decl.parameters),
                "type" => decl.ty, "default" => Maybe(decl.default),
                "clause-params" => List(decl.clause_params), "constraints" => List(decl.clause_constraints)),
            Self::TypeAlias(ty, parameters, constraints) => {
                form!(f, "type-alias", "parameters" => List(parameters), "constraints" => List(constraints), "type" => ty)
            }
            Self::Newtype(ty, parameters, constraints) => {
                form!(f, "newtype", "parameters" => List(parameters), "constraints" => List(constraints), "representation" => ty)
            }
            Self::Use(path) => form!(f, "use", "import" => path),
            Self::Const(ty, value) => form!(f, "const", "type" => ty, "value" => value),
            Self::Invalid => form!(f, "invalid"),
        }
    }
}

impl Display for TestTemplatePort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        form!(f, "test-template-port", "name" => Name(self.name), "binding" => Id(self.binding), "formal" => Maybe(self.formal.map(Id)))
    }
}

impl Display for crate::item::FunctionTemplateInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        form!(f, "function-template-input", "name" => Name(self.name), "binding" => Id(self.binding), "formal" => Maybe(self.formal.map(Id)))
    }
}

impl Display for EffectModifiers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        form!(f, "effect-modifiers", "async" => self.is_async, "inline" => self.is_inline, "direct-passing" => self.is_direct_passing)
    }
}

impl Display for EffectGroupDef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        form!(f, "effect-group", "operations" => Owners(&self.operations),
            "clause-params" => List(self.clause_params), "constraints" => List(self.clause_constraints))
    }
}

impl Display for FnTypeModifiers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        form!(f, "fn-type-modifiers",
            "pure" => self.is_pure,
            "comptime" => self.is_comptime,
            "inline" => self.is_inline,
            "unsafe" => self.is_unsafe,
            "spec" => self.is_spec,
            "verified" => self.is_verified)
    }
}

impl Display for FnModifiers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        form!(f, "modifiers", "test" => self.is_test, "test-template" => self.is_test_template, "function-template" => self.is_function_template, "inline" => self.is_inline, "quote" => self.is_quote, "normal-form-head" => self.is_normal_form_head, "pure" => self.is_pure, "comptime" => self.is_comptime, "unsafe" => self.is_unsafe)
    }
}

impl Display for FnSig<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        form!(f, "fn-signature", "params" => List(self.params), "return-type" => Maybe(self.return_ty),
            "return-bind" => Maybe(self.return_bind.map(Name)), "return-binding" => Maybe(self.return_binding), "modifiers" => self.modifiers,
            "clause-params" => List(self.clause_params), "template-body-binding" => Maybe(self.test_template_body().ok().flatten().map(|parameter| parameter.hir_id)),
            "constraints" => List(self.clause_constraints), "span" => Location(self.span))
    }
}

impl Display for NFSig<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        form!(f, "nf-signature", "params" => List(self.params), "return-type" => Maybe(self.return_ty),
            "return-bind" => Maybe(self.return_bind.map(Name)), "clause-params" => List(self.clause_params),
            "constraints" => List(self.clause_constraints), "span" => Location(self.span))
    }
}

impl Display for FnParam<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        form!(f, "fn-param", "id" => Id(self.hir_id), "name" => Name(self.name), "index" => self.index, "type" => self.ty, "span" => Location(self.span))
    }
}

impl Display for TyParam<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        form!(f, "type-param", "id" => Id(self.hir_id), "kind" => self.kind, "flags" => self.flags, "span" => Location(self.span))
    }
}

impl Display for TyParamKind<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PositionalDependencyCatched(name, ty) => {
                form!(f, "positional-dependency-catched", "name" => Name(*name), "type" => ty)
            }
            Self::Positional(ty) => form!(f, "positional", "type" => ty),
            Self::Optional(name, ty, default) => {
                form!(f, "optional", "name" => Name(*name), "type" => ty, "default" => default)
            }
            Self::Varadic(name, ty) => form!(f, "variadic", "name" => Name(*name), "type" => ty),
            Self::Itself { is_ref } => form!(f, "itself", "ref" => is_ref),
        }
    }
}

impl Display for StructDef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        form!(f, "struct-definition", "fields" => List(self.fields), "clause-params" => List(self.clause_params), "constraints" => List(self.clause_constraints), "nested-items" => Owners(&self.nested_items))
    }
}

impl Display for FieldDef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        form!(f, "field", "id" => Id(self.hir_id), "name" => Name(self.ident), "type" => self.ty, "default" => Maybe(self.default), "span" => Location(self.span))
    }
}

impl Display for EnumDef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        form!(f, "enum-definition", "intrinsic" => self.is_intrinsic, "variants" => List(self.variants), "configurations" => List(&self.configurations), "clause-params" => List(self.clause_params), "constraints" => List(self.clause_constraints), "nested-items" => Owners(&self.nested_items))
    }
}

impl Display for Configuration<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        form!(f, "configuration", "name" => Name(self.ident), "value" => self.value, "span" => Location(self.span))
    }
}

impl Display for Variant<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        form!(f, "variant", "id" => Id(self.hir_id), "name" => Name(self.ident), "kind" => self.kind, "span" => Location(self.span))
    }
}

impl Display for VariantKind<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unit => form!(f, "unit"),
            Self::Pattern(pat) => form!(f, "pattern", "pattern" => pat),
            Self::Const(value) => form!(f, "const", "value" => value),
            Self::Tuple(types) => form!(f, "tuple", "types" => List(types)),
            Self::Struct(fields) => form!(f, "struct", "fields" => List(fields)),
            Self::SubEnum(variants) => form!(f, "sub-enum", "variants" => List(variants)),
        }
    }
}

impl Display for ImplDef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        form!(f, "impl-definition", "self-type" => self.self_ty, "trait" => Maybe(self.trait_ref),
            "clause-params" => List(self.clause_params), "constraints" => List(self.clause_constraints), "items" => Owners(&self.items))
    }
}

impl Display for TraitDef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        form!(f, "trait-definition", "super-traits" => Maybe(self.super_traits), "clause-params" => List(self.clause_params), "constraints" => List(self.clause_constraints), "items" => Owners(&self.items))
    }
}

impl Display for UsePath<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        form!(f, "use-path", "path" => QualifiedPath(&self.path), "kind" => self.kind, "span" => Location(self.span))
    }
}

impl Display for UseKind<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Simple => form!(f, "simple"),
            Self::Glob => form!(f, "glob"),
            Self::Group(paths) => form!(f, "group", "paths" => List(paths)),
            Self::Multi(names) => {
                f.write_str("(multi (names (")?;
                for (i, name) in names.iter().enumerate() {
                    if i != 0 {
                        f.write_str(" ")?;
                    }
                    write!(f, "{}", Name(*name))?;
                }
                f.write_str(")))")
            }
            Self::Alias(name) => form!(f, "alias", "name" => Name(*name)),
        }
    }
}

struct QualifiedPath<'a, 'hir>(&'a Path<'hir>);
impl Display for QualifiedPath<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        struct Anchor(PathAnchor);
        impl Display for Anchor {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                match self.0 {
                    PathAnchor::Local => form!(f, "local"),
                    PathAnchor::Super(levels) => form!(f, "super", "levels" => levels),
                    PathAnchor::Package => form!(f, "package"),
                }
            }
        }
        form!(f, "path", "anchor" => Anchor(self.0.anchor), "segments" => List(self.0.segments),
            "resolution" => Maybe(self.0.res.map(Definition)), "span" => Location(self.0.span))
    }
}

impl Display for PathSegment<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        form!(f, "segment", "name" => Name(self.ident), "args" => List(self.args))
    }
}

impl Display for Arg<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Positional(expr) => form!(f, "positional", "value" => expr),
            Self::Named(name, expr) => form!(f, "named", "name" => Name(*name), "value" => expr),
            Self::Expand(expr) => form!(f, "expand", "value" => expr),
            Self::Implicit(expr) => form!(f, "implicit", "value" => expr),
            Self::PostLambda(expr) => form!(f, "post-lambda", "value" => expr),
        }
    }
}

impl Display for ClauseParam<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        form!(f, "clause-param", "id" => Id(self.hir_id), "definition" => Maybe(self.definition.map(Definition)), "name" => Name(self.name), "kind" => self.kind, "span" => Location(self.span))
    }
}

impl Display for ClauseParamKind<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Type(name) => form!(f, "type", "name" => Name(*name)),
            Self::Positional(name, ty) => {
                form!(f, "positional", "name" => Name(*name), "type" => ty)
            }
            Self::Optional(name, ty, default) => {
                form!(f, "optional", "name" => Name(*name), "type" => ty, "default" => default)
            }
            Self::Varadic(name, ty) => form!(f, "variadic", "name" => Name(*name), "type" => ty),
            Self::TraitBound(name, bound) => {
                form!(f, "trait-bound", "name" => Name(*name), "bound" => bound)
            }
            Self::Quote(inner) => form!(f, "quote", "parameter" => inner),
        }
    }
}

impl Display for ClauseConstraint<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        form!(f, "constraint", "label" => Maybe(self.label.map(Name)), "id" => Id(self.hir_id), "kind" => self.kind, "span" => Location(self.span))
    }
}

impl Display for ClauseConstraintKind<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Requires(expr) => form!(f, "requires", "value" => expr),
            Self::Ensures(expr) => form!(f, "ensures", "value" => expr),
            Self::Decreases(expr) => form!(f, "decreases", "value" => expr),
            Self::Outcome => form!(f, "outcome"),
        }
    }
}

impl Display for Body<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        form!(f, "body", "params" => List(self.params), "value" => self.value)
    }
}

impl Display for BodyParam<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        form!(f, "body-param", "id" => Id(self.hir_id), "name" => Name(self.name), "type" => self.ty, "span" => Location(self.span))
    }
}

impl Display for Expr<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        form!(f, "expr", "id" => Id(self.hir_id), "kind" => self.kind, "span" => Location(self.span))
    }
}

impl Display for ExprKind<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lit(lit) => form!(f, "literal", "value" => lit),
            Self::ContextualPatternType(subject, bindings) => {
                write!(f, "(contextual-pattern-type (subject {subject}) (outputs")?;
                for entry in bindings {
                    write!(
                        f,
                        " (binding (name {}) (id {}) (type {}))",
                        Name(entry.name),
                        Id(entry.hir_id),
                        entry.ty
                    )?;
                }
                f.write_str("))")
            }
            Self::CodePatternQuote(pattern, substitutions) => {
                write!(f, "(code-pattern-quote (pattern {pattern}) (substitutions")?;
                for entry in substitutions {
                    write!(
                        f,
                        " (substitute (port {}) (binding {}))",
                        Name(entry.port),
                        entry.binding
                    )?;
                }
                f.write_str("))")
            }
            Self::ContextualExprType(bindings, result) => {
                f.write_str("(contextual-expr-type (context")?;
                for entry in bindings {
                    write!(
                        f,
                        " (binding (name {}) (id {}) (type {}) (default {}))",
                        Name(entry.name),
                        Id(entry.hir_id),
                        entry.ty,
                        Maybe(entry.default)
                    )?;
                }
                write!(f, ") (result {}))", result)
            }
            Self::CodeQuote(body, substitutions) | Self::CodeSplice(body, substitutions) => {
                write!(
                    f,
                    "({} (body {}) (substitutions",
                    if matches!(self, Self::CodeQuote(..)) {
                        "code-quote"
                    } else {
                        "code-splice"
                    },
                    body
                )?;
                for entry in substitutions {
                    write!(
                        f,
                        " (substitute (port {}) (binding {}))",
                        Name(entry.port),
                        entry.binding
                    )?;
                }
                f.write_str("))")
            }
            Self::LiteralConversion { literal, suffix } => {
                form!(f, "literal-conversion", "literal" => literal, "suffix" => Name(*suffix))
            }
            Self::Path(path) => form!(f, "path-expr", "path" => QualifiedPath(path)),
            Self::UnresolvedIdent(name) => form!(f, "unresolved-ident", "name" => Quoted(name)),
            Self::TacticParameter(index) => form!(f, "tactic-parameter", "index" => index),
            Self::TacticArgument(value) => form!(f, "tactic-argument", "value" => value),
            Self::PostLambdaBody(body) => form!(f, "post-lambda-body", "body" => body),
            Self::ResolvedLocal(name, id) => {
                form!(f, "resolved-local", "name" => Quoted(name), "target" => Id(*id))
            }
            Self::ResolvedFnParam(name, id) => {
                form!(f, "resolved-fn-param", "name" => Quoted(name), "target" => Id(*id))
            }
            Self::ResolvedClauseParam(name, id) => {
                form!(f, "resolved-clause-param", "name" => Quoted(name), "target" => Definition(*id))
            }
            Self::ResolvedDef {
                name,
                has_clause_param,
                def_id,
            } => {
                form!(f, "resolved-def", "name" => Quoted(name), "has-clause-param" => has_clause_param, "target" => Definition(*def_id))
            }
            Self::ResolvedLoopLabel(name, id) => {
                form!(f, "resolved-loop-label", "name" => Quoted(name), "target" => Id(*id))
            }
            Self::SelfValue => form!(f, "self-value"),
            Self::Range {
                start,
                end,
                inclusive,
            } => {
                form!(f, "range-expr", "start" => Maybe(*start), "end" => Maybe(*end), "inclusive" => inclusive)
            }
            Self::Index(base, index) => form!(f, "index", "base" => base, "index" => index),
            Self::Application(callee, args) => {
                form!(f, "application", "callee" => callee, "args" => List(args))
            }
            Self::CollectedProperties(fields) => {
                form!(f, "collected-properties", "fields" => List(fields))
            }
            Self::ExtendedApplication(callee, args) => {
                form!(f, "extended-application", "callee" => callee, "args" => List(args))
            }
            Self::NFApplication(callee, args) => {
                form!(f, "nf-application", "callee" => callee, "args" => List(args))
            }
            Self::Binary(op, left, right) => {
                form!(f, "binary", "op" => op, "left" => left, "right" => right)
            }
            Self::Unary(op, value) => form!(f, "unary", "op" => op, "value" => value),
            Self::If(cond, then, otherwise) => {
                form!(f, "if", "condition" => cond, "then" => then, "else" => Maybe(*otherwise))
            }
            Self::When(arms) => form!(f, "when", "arms" => List(arms)),
            Self::UnsafeBlock(block) => form!(f, "unsafe-block-expr", "block" => block),
            Self::Block(block) => form!(f, "block-expr", "block" => block),
            Self::For {
                label,
                pat,
                guard,
                iter,
                body,
            } => form!(f, "for", "label" => Maybe(label.map(Name)),
                "pattern" => pat, "guard" => Maybe(*guard), "iterable" => iter, "body" => body),
            Self::Loop(block) => form!(f, "loop", "body" => block),
            Self::EffectRequest(value) => form!(f, "effect-request", "computation" => value),
            Self::EffectHandle(value, arms) => {
                form!(f, "effect-handle", "computation" => value, "arms" => List(arms))
            }
            Self::Match(value, arms) => form!(f, "match", "value" => value, "arms" => List(arms)),
            Self::Assign(place, value) => form!(f, "assign", "place" => place, "value" => value),
            Self::AssignOp(op, place, value) => {
                form!(f, "assign-op", "op" => op, "place" => place, "value" => value)
            }
            Self::Assumes(value) => form!(f, "assumes", "condition" => value),
            Self::Defer(value) => form!(f, "defer", "value" => value),
            Self::InlineControl(value) => form!(f, "inline-control", "value" => value),
            Self::RefinedType { requirement, base } => {
                form!(f, "refined-type", "requirement" => requirement, "base" => base)
            }
            Self::RefinementSubject => write!(f, "(refinement-subject)"),
            Self::Return(value) => form!(f, "return", "value" => Maybe(*value)),
            Self::Resume(value) => form!(f, "resume", "value" => Maybe(*value)),
            Self::BreakTo(target) => form!(f, "break-to", "target" => Id(*target)),
            Self::ContinueTo(target) => form!(f, "continue-to", "target" => Id(*target)),
            Self::Break(label) => form!(f, "break", "label" => Name(*label)),
            Self::Continue(label) => form!(f, "continue", "label" => Name(*label)),
            Self::Projection(base, field) => {
                form!(f, "projection", "base" => base, "field" => Name(*field))
            }
            Self::Tuple(values) => form!(f, "tuple", "values" => List(values)),
            Self::List(values) => form!(f, "list", "values" => List(values)),
            Self::Object(entries) => form!(f, "object", "entries" => List(entries)),
            Self::Ref(value) => form!(f, "ref", "value" => value),
            Self::Deref(value) => form!(f, "deref", "value" => value),
            Self::ErrorNew(value) => form!(f, "error-new", "value" => value),
            Self::ErrorPropagate(value) => form!(f, "error-propagate", "value" => value),
            Self::ErrorHandle {
                value,
                before,
                arms,
                after,
            } => form!(f, "error-handle", "value" => value,
                "before" => Maybe(*before), "arms" => List(arms), "after" => Maybe(*after)),
            Self::Closure(params, ret, body) => {
                form!(f, "closure", "params" => List(params), "return-type" => Maybe(*ret), "body" => body)
            }
            Self::Cast(value, ty) => form!(f, "cast", "value" => value, "type" => ty),
            Self::Let(decl) => form!(f, "let", "declaration" => decl),
            Self::Semi(value) => form!(f, "semi", "value" => value),
            Self::Item(owner) => form!(f, "item-ref", "owner" => Owner(*owner)),
            Self::Undefined => form!(f, "undefined"),
            Self::OptionPropagate(value) => form!(f, "option-propagate", "value" => value),
            Self::Null => form!(f, "null"),
            Self::Unit => form!(f, "unit"),
            Self::InlineIf {
                cond,
                then_expr,
                else_expr,
            } => {
                form!(f, "inline-if", "condition" => cond, "then" => then_expr, "else" => Maybe(*else_expr))
            }
            Self::InlineMatch(arms) => form!(f, "inline-match", "arms" => List(arms)),
            Self::InlineFor {
                label,
                pat,
                iter,
                body,
            } => {
                form!(f, "inline-for", "label" => Maybe(label.map(Name)), "pattern" => pat, "iterator" => iter, "body" => body)
            }
            Self::TyClosure(context, body) => {
                form!(f, "ty-closure", "context" => context, "body" => body)
            }
            Self::TyClosureEnvironment(bindings) => {
                form!(f, "closure-environment", "bindings" => List(bindings))
            }
            Self::TyPtr(ty) => form!(f, "ty-ptr", "type" => ty),
            Self::TyOptional(ty) => form!(f, "ty-optional", "type" => ty),
            Self::TyComptimeFn(params) => form!(f, "ty-comptime-fn", "params" => List(params)),
            Self::TyFn(params) => form!(f, "ty-fn", "params" => List(params)),
            Self::TyModifiedFn(modifiers, params) => {
                form!(f, "ty-modified-fn", "modifiers" => modifiers, "params" => List(params))
            }
            Self::TyNFFn(params) => form!(f, "ty-nf-fn", "params" => List(params)),
            Self::TyFnArrow(params, ret) => {
                form!(f, "ty-fn-arrow", "params" => params, "return-type" => ret)
            }
            Self::ReachabilityType => form!(f, "reachability-type"),
            Self::ErrorQualifiedType(errors, result) => {
                form!(f, "error-qualified-type", "errors" => errors, "result" => result)
            }
            Self::EffectQualifiedType(effects, result) => {
                form!(f, "effect-qualified-type", "effects" => effects, "result" => result)
            }
            Self::TyPlaceholder => form!(f, "ty-placeholder"),
            Self::TyNoReturn => form!(f, "ty-no-return"),
            Self::TyVoid => form!(f, "ty-void"),
            Self::DeliveredType(ty) => form!(f, "delivered-type", "type" => ty),
            Self::TyAny => form!(f, "ty-any"),
            Self::TyType => form!(f, "ty-type"),
            Self::TySelf => form!(f, "ty-self"),
            Self::TermTypedWith => form!(f, "term-typed-with"),
            Self::TraitBound(subject, interface) => {
                form!(f, "trait-bound", "subject" => subject, "interface" => interface)
            }
            Self::LambdaBound => form!(f, "lambda-bound"),
            Self::TermTraitBound => form!(f, "term-trait-bound"),
            Self::Implication => form!(f, "implication"),
            Self::Subtype => form!(f, "subtype"),
            Self::Forall => form!(f, "forall"),
            Self::Exist => form!(f, "exist"),
            Self::Builtin(name) => form!(f, "builtin", "name" => Quoted(name)),
            Self::TakeOld(value) => form!(f, "take-old", "value" => value),
            Self::TakeIntrinsic(value) => form!(f, "take-intrinsic", "tag" => value),
            Self::TakeTagName(value) => form!(f, "take-tag-name", "value" => value),
            Self::TakeTupleEnumerate(value) => form!(f, "take-tuple-enumerate", "value" => value),
            Self::TakeBitVec(value) => form!(f, "take-bit-vec", "value" => value),
            Self::TypeOf(value) => form!(f, "type-of", "value" => value),
            Self::MultiDeref(value) => form!(f, "multi-deref", "value" => value),
            Self::Invalid => form!(f, "invalid"),
        }
    }
}

impl Display for Lit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        form!(f, "lit", "kind" => self.kind, "span" => Location(self.span))
    }
}

impl Display for LitKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Integer(spelling) => form!(f, "integer", "spelling" => Quoted(spelling)),
            Self::Float(spelling) => form!(f, "float", "spelling" => Quoted(spelling)),
            Self::String(value) => form!(f, "string", "value" => Quoted(value)),
            Self::Bool(value) => form!(f, "bool", "value" => value),
            Self::Char(value) => {
                let mut bytes = [0; 4];
                form!(f, "char", "value" => Quoted(value.encode_utf8(&mut bytes)))
            }
            Self::Symbol(value) => form!(f, "symbol", "value" => Quoted(value)),
        }
    }
}

impl Display for Block<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        form!(f, "block", "id" => Id(self.hir_id), "statements" => List(self.stmts), "tail" => Maybe(self.expr), "span" => Location(self.span))
    }
}

impl Display for CondictionArm<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        form!(f, "condition-arm", "id" => Id(self.hir_id), "condition" => Maybe(self.cond), "body" => self.body, "span" => Location(self.span))
    }
}

impl Display for FieldExpr<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        form!(f, "field-expr", "name" => Name(self.ident), "value" => self.expr, "span" => Location(self.span))
    }
}

impl Display for ClosureParam<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        form!(f, "closure-param", "id" => Id(self.hir_id), "pattern" => self.pat, "type" => Maybe(self.ty), "span" => Location(self.span))
    }
}

impl Display for LetDecl<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        form!(f, "let-decl", "mutable" => self.mutable, "comptime" => self.comptime, "id" => Id(self.hir_id), "pattern" => self.pattern, "guard" => Maybe(self.guard), "else" => Maybe(self.otherwise), "type" => Maybe(self.ty), "init" => Maybe(self.init), "span" => Location(self.span))
    }
}

impl Display for Pattern<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        form!(f, "pattern", "id" => Id(self.hir_id), "kind" => self.kind, "span" => Location(self.span))
    }
}

impl Display for PatternKind<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CodeSplice(operand, outputs) => {
                write!(f, "(code-pattern-splice (operand {operand}) (outputs")?;
                for output in outputs {
                    write!(
                        f,
                        " (output (port {}) (name {}) (id {}))",
                        Name(output.port),
                        Name(output.name),
                        Id(output.hir_id)
                    )?;
                }
                f.write_str("))")
            }
            Self::Then(first, value, next) => {
                form!(f, "and-is", "first" => first, "value" => value, "next" => next)
            }
            Self::Guard(pattern, condition) => {
                form!(f, "guard-pattern", "pattern" => pattern, "condition" => condition)
            }
            Self::Wild => form!(f, "wild"),
            Self::Projection(base, field) => {
                form!(f, "projection", "base" => base, "field" => Name(*field))
            }
            Self::Binding(mode, name, subpat) => {
                form!(f, "binding", "mode" => mode, "name" => Name(*name), "pattern" => Maybe(*subpat))
            }
            Self::Const(value) => form!(f, "const", "value" => value),
            Self::Comptime(value) => form!(f, "comptime", "value" => value),
            Self::Tuple(patterns) => form!(f, "tuple", "patterns" => List(patterns)),
            Self::Struct(base, fields, rest) => {
                form!(f, "struct", "base" => base, "fields" => List(fields), "rest" => rest)
            }
            Self::List(patterns, rest) => {
                form!(f, "list", "patterns" => List(patterns), "rest" => Maybe(*rest))
            }
            Self::NormalForm(head, arguments) => {
                write!(f, "(normal-form-pattern (head {head}) (arguments")?;
                for argument in *arguments {
                    f.write_str(" (argument (name")?;
                    if let Some(name) = argument.name {
                        write!(f, " {}", Name(name))?;
                    }
                    write!(f, ") (pattern {}))", argument.pattern)?;
                }
                f.write_str("))")
            }
            Self::AppTuple(base, patterns) => {
                form!(f, "app-tuple", "base" => base, "patterns" => List(patterns))
            }
            Self::AppStruct(base, fields) => {
                form!(f, "app-struct", "base" => base, "fields" => List(fields))
            }
            Self::OptionSome(pattern) => form!(f, "option-some", "pattern" => pattern),
            Self::OptionNull => form!(f, "option-null"),
            Self::ErrorOk(pattern) => form!(f, "error-ok", "pattern" => pattern),
            Self::ErrorErr(pattern) => form!(f, "error-err", "pattern" => pattern),
            Self::Or(patterns) => form!(f, "or", "patterns" => List(patterns)),
            Self::Not(pattern) => form!(f, "not", "pattern" => pattern),
            Self::Ref(pattern) => form!(f, "ref", "pattern" => pattern),
            Self::Range(start, end, bound) => {
                form!(f, "range", "start" => Maybe(*start), "end" => Maybe(*end), "bound" => bound)
            }
            Self::Async => form!(f, "async"),
            Self::BitVec { radix, segments } => {
                form!(f,"bit-vec","radix"=>radix,"segments"=>List(segments))
            }
            Self::Invalid => form!(f, "invalid"),
        }
    }
}

impl Display for crate::pattern::BitSegment<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Literal { spelling, span } => {
                form!(f,"literal","spelling"=>Quoted(spelling),"span"=>Location(*span))
            }
            Self::Typed { pattern, ty } => form!(f,"typed","pattern"=>pattern,"type"=>ty),
            Self::Computed(value) => form!(f,"computed","value"=>value),
        }
    }
}

impl Display for BindingMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ByValue => f.write_str("by-value"),
            Self::ByRef => f.write_str("by-ref"),
        }
    }
}

impl Display for BoundType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Inclusive => f.write_str("inclusive"),
            Self::Exclusive => f.write_str("exclusive"),
        }
    }
}

impl Display for PatternArm<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        form!(f, "pattern-arm", "id" => Id(self.hir_id), "pattern" => self.pat, "guard" => Maybe(self.guard), "body" => self.body, "span" => Location(self.span))
    }
}

impl Display for FieldPat<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        form!(f, "field-pattern", "name" => Name(self.ident), "pattern" => self.pat, "span" => Location(self.span))
    }
}

impl Display for crate::expr::ClosureEnvironmentBinding<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        form!(f, "environment-binding", "name" => Name(self.name), "type" => self.ty, "source" => Maybe(self.value))
    }
}

impl Display for TestTemplateInterface<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        form!(f, "test-template-interface", "parameter" => Id(self.parameter), "type" => self.ty)
    }
}
