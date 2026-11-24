//! Text backing-region dependencies. This analysis does not grant memory access
//! or prove that a caller-supplied region is live; it preserves that obligation.
mod transfers;

use super::*;
use std::collections::VecDeque;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TextRegionOrigin {
    Static,
    Parameter {
        parameter: usize,
        fields: Vec<usize>,
    },
}

impl fmt::Display for TextRegionOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Static => f.write_str("(static-text-region)"),
            Self::Parameter { parameter, fields } => {
                write!(f, "(parameter-text-region {parameter} (fields")?;
                display_owned_path(fields, f)?;
                f.write_str("))")
            }
        }
    }
}

type TextFields = BTreeMap<Vec<usize>, BTreeSet<TextRegionOrigin>>;

type CodeFields = BTreeMap<Vec<usize>, Option<CallbackKey>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum DescriptorRoot {
    Local(LocalId),
    Frame(usize),
}

impl std::hash::Hash for DescriptorRoot {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            Self::Local(id) => {
                0u8.hash(state);
                id.owner.hash(state);
                id.index.hash(state);
            }
            Self::Frame(id) => {
                1u8.hash(state);
                id.hash(state);
            }
        }
    }
}

type DescriptorTargets = BTreeSet<(DescriptorRoot, Vec<usize>)>;

type DescriptorPlaces = BTreeMap<Vec<usize>, Option<DescriptorTargets>>;

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Regions {
    text: TextFields,
    // None is unknown code or incompatible closure environments, not absence.
    code: CodeFields,
    places: DescriptorPlaces,
}

impl Regions {
    fn new() -> Self {
        Self::default()
    }
}

impl std::ops::Deref for Regions {
    type Target = TextFields;

    fn deref(&self) -> &TextFields {
        &self.text
    }
}

impl std::ops::DerefMut for Regions {
    fn deref_mut(&mut self) -> &mut TextFields {
        &mut self.text
    }
}

impl From<TextFields> for Regions {
    fn from(text: TextFields) -> Self {
        Self {
            text,
            code: BTreeMap::new(),
            places: BTreeMap::new(),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
struct Summary {
    declaration: DeclarationId,
    arguments: Vec<Term>,
    reads: BTreeSet<TextRegionOrigin>,
    returned: Option<Regions>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Locals {
    bindings: BTreeMap<LocalId, Regions>,
    frames: BTreeMap<usize, Regions>,
}

impl Locals {
    fn new() -> Self {
        Self::default()
    }

    fn contains_storage(&self, root: &DescriptorRoot) -> bool {
        self.get_storage(root).is_some()
    }

    fn get_storage(&self, root: &DescriptorRoot) -> Option<&Regions> {
        match root {
            DescriptorRoot::Local(id) => self.bindings.get(id),
            DescriptorRoot::Frame(id) => self.frames.get(id),
        }
    }

    fn get_storage_mut(&mut self, root: &DescriptorRoot) -> Option<&mut Regions> {
        match root {
            DescriptorRoot::Local(id) => self.bindings.get_mut(id),
            DescriptorRoot::Frame(id) => self.frames.get_mut(id),
        }
    }
}

impl std::ops::Deref for Locals {
    type Target = BTreeMap<LocalId, Regions>;

    fn deref(&self) -> &Self::Target {
        &self.bindings
    }
}

impl std::ops::DerefMut for Locals {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.bindings
    }
}

fn remap_descriptor_roots(value: &mut Regions, mapping: &BTreeMap<DescriptorRoot, DescriptorRoot>) {
    for targets in value.places.values_mut().flatten() {
        *targets = std::mem::take(targets)
            .into_iter()
            .map(|(root, path)| (mapping.get(&root).copied().unwrap_or(root), path))
            .collect();
    }
}

/// Conservative type-level predicate for whether values of `ty` (or storage
/// reachable through pointers inside `ty`) can hold a `str` view or callable.
/// Exhaustion propagates as `Err`; any unknown/opaque type fails closed (`true`).
pub(super) fn type_may_hold_text_or_callable(
    package: &Package,
    ty: &Term,
    fuel: &mut usize,
) -> Result<bool, String> {
    fn walk(
        package: &Package,
        ty: &Term,
        fuel: &mut usize,
        depth: usize,
        visiting: &mut Vec<Term>,
        clean_false: &mut BTreeSet<OrderedTerm>,
    ) -> Result<(bool, bool), String> {
        if depth >= 128 {
            return Ok((true, false));
        }
        charge(fuel, 1)?;
        if matches!(
            ty,
            Term::Primitive(Primitive::Str | Primitive::Type | Primitive::Object)
                | Term::Function { .. }
                | Term::UnsafeFunction(_)
                | Term::Closure { .. }
        ) {
            return Ok((true, false));
        }
        if matches!(ty, Term::Primitive(_)) {
            return Ok((false, false));
        }
        if visiting.contains(ty) {
            return Ok((false, true));
        }
        let ordered_ty = OrderedTerm(ty.clone());
        if clean_false.contains(&ordered_ty) {
            return Ok((false, false));
        }
        visiting.push(ty.clone());
        let outcome = (|| {
            match pointer_representation(Some(package), ty, fuel) {
                Ok(Some((_, pointee))) => {
                    return walk(package, &pointee, fuel, depth + 1, visiting, clean_false);
                }
                Ok(None) => {}
                Err(error) if error.contains("exhausted") => return Err(error),
                Err(_) => return Ok((true, false)),
            }
            if intrinsic_pointer_value(ty) {
                return Ok((false, false));
            }
            let is_enum = matches!(ty, Term::ErrorResult { .. } | Term::Optional(_))
                || matches!(
                    ty,
                    Term::Nominal { declaration, .. }
                        if package.enums.iter().any(|e| e.declaration == *declaration)
                );
            let mut hit_cycle = false;
            if is_enum {
                let variants = match owned_variant_fields(Some(package), ty, fuel) {
                    Ok(variants) => variants,
                    Err(error) if error.contains("exhausted") => return Err(error),
                    Err(_) => return Ok((true, false)),
                };
                for fields in variants {
                    for field in fields {
                        let (may_hold, field_cycle) =
                            walk(package, &field, fuel, depth + 1, visiting, clean_false)?;
                        if may_hold {
                            return Ok((true, false));
                        }
                        hit_cycle |= field_cycle;
                    }
                }
            } else {
                let fields = match owned_fields(Some(package), ty, fuel) {
                    Ok(fields) => fields,
                    Err(error) if error.contains("exhausted") => return Err(error),
                    Err(_) => return Ok((true, false)),
                };
                for field in fields {
                    let (may_hold, field_cycle) =
                        walk(package, &field, fuel, depth + 1, visiting, clean_false)?;
                    if may_hold {
                        return Ok((true, false));
                    }
                    hit_cycle |= field_cycle;
                }
            }
            Ok((false, hit_cycle))
        })();
        visiting.pop();
        if let Ok((false, false)) = &outcome {
            clean_false.insert(ordered_ty);
        }
        outcome
    }

    let mut visiting = Vec::new();
    let mut clean_false = BTreeSet::new();
    Ok(walk(package, ty, fuel, 0, &mut visiting, &mut clean_false)?.0)
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct StorageKey {
    declaration: DeclarationId,
    arguments: Vec<Term>,
    actual: BTreeMap<usize, Regions>,
    storage: BTreeMap<usize, Regions>,
    readonly: Option<&'static str>,
}

impl Ord for StorageKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.declaration
            .cmp(&other.declaration)
            .then_with(|| cmp_term_slice(&self.arguments, &other.arguments))
            .then_with(|| self.actual.cmp(&other.actual))
            .then_with(|| self.storage.cmp(&other.storage))
            .then_with(|| self.readonly.cmp(&other.readonly))
    }
}

impl PartialOrd for StorageKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Default, PartialEq, Eq)]
struct StorageSummary {
    returned: Option<Regions>,
    storage: Option<BTreeMap<usize, Regions>>,
    reads: BTreeSet<TextRegionOrigin>,
}

struct StorageEntry {
    key: StorageKey,
    summary: StorageSummary,
    cost: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum CallbackKey {
    Named(iir::InstanceKey),
    Closure(usize),
    Alternatives(Vec<iir::InstanceKey>),
    ClosureAlternatives(Vec<usize>),
}

impl Ord for CallbackKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let rank = |value: &Self| match value {
            Self::Named(_) => 0u8,
            Self::Closure(_) => 1,
            Self::Alternatives(_) => 2,
            Self::ClosureAlternatives(_) => 3,
        };
        rank(self)
            .cmp(&rank(other))
            .then_with(|| match (self, other) {
                (Self::Named(a), Self::Named(b)) => cmp_instance_key(a, b),
                (Self::Closure(a), Self::Closure(b)) => a.cmp(b),
                (Self::Alternatives(a), Self::Alternatives(b)) => {
                    cmp_slice_by(a, b, cmp_instance_key)
                }
                (Self::ClosureAlternatives(a), Self::ClosureAlternatives(b)) => a.cmp(b),
                _ => std::cmp::Ordering::Equal,
            })
    }
}

impl PartialOrd for CallbackKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for CallbackKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Named(key) => key.fmt(f),
            Self::Closure(id) => write!(f, "(closed-text-callback {id})"),
            Self::ClosureAlternatives(ids) => {
                f.write_str("(closed-text-callback-alternatives")?;
                for id in ids {
                    write!(f, " {id}")?;
                }
                f.write_str(")")
            }
            Self::Alternatives(keys) => {
                f.write_str("(text-callback-alternatives")?;
                for key in keys {
                    write!(f, " {key}")?;
                }
                f.write_str(")")
            }
        }
    }
}

#[derive(Clone, PartialEq)]
struct CodeValue {
    expression: std::sync::Arc<Expr>,
    closure: Option<usize>,
}

impl CodeValue {
    fn key(&self) -> Option<CallbackKey> {
        if let Some(id) = self.closure {
            return Some(CallbackKey::Closure(id));
        }
        match &self.expression.kind {
            K::Function(declaration) => Some(CallbackKey::Named(iir::InstanceKey {
                declaration: *declaration,
                arguments: vec![].into(),
            })),
            K::FunctionInstance {
                declaration,
                arguments,
            } => Some(CallbackKey::Named(iir::InstanceKey {
                declaration: *declaration,
                arguments: arguments.clone().into(),
            })),
            _ => None,
        }
    }

    fn new(expression: Expr) -> Self {
        Self {
            expression: std::sync::Arc::new(expression),
            closure: None,
        }
    }
}

struct ClosedClosure {
    expression: std::sync::Arc<Expr>,
    definitions: BTreeMap<LocalId, CodeValue>,
    changed: BTreeSet<LocalId>,
    cost: usize,
}

fn index_code_expression(
    expr: &Expr,
    fuel: &mut usize,
    definitions: &mut BTreeMap<LocalId, CodeValue>,
    seen: &mut BTreeSet<LocalId>,
    changed: &mut BTreeSet<LocalId>,
) -> Result<(), String> {
    match &expr.kind {
        K::Let { binding, value } => {
            if !seen.insert(*binding) {
                changed.insert(*binding);
            }
            if matches!(
                &value.kind,
                K::Closure { .. } | K::Function(_) | K::FunctionInstance { .. } | K::Local(_)
            ) {
                nir::visit::expression(value, fuel, |_, _| Ok(()))?;
                definitions.insert(*binding, CodeValue::new((**value).clone()));
            }
        }
        K::Assign { place, .. } => {
            changed.insert(place.root);
        }
        K::Closure { captures, .. } => {
            for capture in captures {
                if capture.value.is_some() {
                    changed.insert(capture.binding);
                }
            }
        }
        _ => {}
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct SummaryKey {
    instance: iir::InstanceKey,
    callbacks: Vec<(usize, Vec<usize>, CallbackKey)>,
    borrowed_callbacks: Vec<(usize, Vec<usize>, Vec<usize>, CallbackKey)>,
}

impl Ord for SummaryKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        cmp_instance_key(&self.instance, &other.instance)
            .then_with(|| self.callbacks.cmp(&other.callbacks))
            .then_with(|| self.borrowed_callbacks.cmp(&other.borrowed_callbacks))
    }
}

impl PartialOrd for SummaryKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for SummaryKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(text-summary-key {} (callbacks", self.instance)?;
        for (parameter, path, callback) in &self.callbacks {
            write!(f, " (parameter {parameter} (path")?;
            for field in path {
                write!(f, " {field}")?;
            }
            write!(f, ") {callback})")?;
        }
        if !self.borrowed_callbacks.is_empty() {
            f.write_str(") (borrowed-callbacks")?;
            for (parameter, input_fields, path, callback) in &self.borrowed_callbacks {
                write!(f, " (parameter {parameter} (input-fields")?;
                for field in input_fields {
                    write!(f, " {field}")?;
                }
                f.write_str(") (path")?;
                for field in path {
                    write!(f, " {field}")?;
                }
                write!(f, ") {callback})")?;
            }
        }
        f.write_str("))")
    }
}

struct LoopRegions {
    identity: LocalId,
    bindings: Vec<LocalId>,
    exits: Option<Locals>,
    backedges: Option<Locals>,
}

struct ExitRegions {
    bindings: Vec<LocalId>,
    locals: Option<Locals>,
}

fn join_locals(
    target: &mut Option<Locals>,
    source: &Locals,
    bindings: &[LocalId],
    fuel: &mut usize,
) -> Result<(), String> {
    let target = target.get_or_insert_with(Locals::new);
    for (id, value) in &source.frames {
        charge_regions(value, fuel)?;
        let mut joined = target.frames.remove(id);
        merge(&mut joined, Some(value.clone()), fuel)?;
        target.frames.insert(
            *id,
            joined.ok_or("text region frame join produced no state")?,
        );
    }
    for binding in bindings {
        charge(fuel, 1)?;
        let value = source
            .get(binding)
            .ok_or("text loop lost an outer binding")?;
        charge_regions(value, fuel)?;
        let mut joined = target.remove(binding);
        merge(&mut joined, Some(value.clone()), fuel)?;
        target.insert(
            *binding,
            joined.ok_or("text region local join produced no state")?,
        );
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextRegionSummary {
    pub declaration: DeclarationId,
    pub arguments: Vec<Term>,
    /// Character reads in explicit expressions and named callees. Cleanup
    /// footprints and region liveness must be composed separately; this set is
    /// not a complete memory-access permission or a cleanup certificate.
    pub reads: BTreeSet<TextRegionOrigin>,
    /// None means no normal return. Empty maps/sets never mean static storage;
    /// guarded leaves can be absent on a particular returned variant.
    pub returned: Option<BTreeMap<Vec<usize>, BTreeSet<TextRegionOrigin>>>,
}

impl fmt::Display for TextRegionSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(text-region-summary (declaration {}) (arguments",
            self.declaration
        )?;
        for argument in &self.arguments {
            write!(f, " {argument}")?;
        }
        f.write_str(") (reads")?;
        for origin in &self.reads {
            write!(f, " {origin}")?;
        }
        f.write_str(") (returned")?;
        match &self.returned {
            None => f.write_str(" (no-return)")?,
            Some(fields) => {
                for (path, origins) in fields {
                    f.write_str(" (view (fields")?;
                    display_owned_path(path, f)?;
                    f.write_str(") (origins")?;
                    for origin in origins {
                        write!(f, " {origin}")?;
                    }
                    f.write_str("))")?;
                }
            }
        }
        f.write_str("))")
    }
}

fn charge(fuel: &mut usize, amount: usize) -> Result<(), String> {
    *fuel = fuel
        .checked_sub(amount)
        .ok_or("text region inference budget exhausted")?;
    Ok(())
}

fn charge_callback(code: &CallbackKey, fuel: &mut usize) -> Result<(), String> {
    let keys = match code {
        CallbackKey::Named(key) => std::slice::from_ref(key),
        CallbackKey::Alternatives(keys) => keys.as_slice(),
        CallbackKey::ClosureAlternatives(ids) => {
            charge(fuel, ids.len().saturating_add(1))?;
            return Ok(());
        }
        CallbackKey::Closure(_) => {
            charge(fuel, 1)?;
            return Ok(());
        }
    };
    for key in keys {
        charge(fuel, 1)?;
        for argument in key.arguments.iter() {
            bound_type(argument, fuel)?;
        }
    }
    Ok(())
}

fn charge_regions(regions: &Regions, fuel: &mut usize) -> Result<(), String> {
    for (path, place) in &regions.places {
        charge(fuel, path.len().saturating_add(1))?;
        if let Some(targets) = place {
            for (_, fields) in targets {
                charge(fuel, fields.len().saturating_add(1))?;
            }
        }
    }
    for (path, code) in &regions.code {
        charge(fuel, path.len().saturating_add(1))?;
        if let Some(code) = code {
            charge_callback(code, fuel)?;
        }
    }
    for (path, origins) in &regions.text {
        charge(fuel, path.len().saturating_add(1))?;
        for origin in origins {
            charge(
                fuel,
                match origin {
                    TextRegionOrigin::Static => 1,
                    TextRegionOrigin::Parameter { fields, .. } => fields.len().saturating_add(1),
                },
            )?;
        }
    }
    Ok(())
}

fn merge_known(
    target: &mut CodeFields,
    source: CodeFields,
    fuel: &mut usize,
) -> Result<(), String> {
    for (path, code) in source {
        charge(fuel, path.len().saturating_add(1))?;
        let Some(previous) = target.get_mut(&path) else {
            target.insert(path, code);
            continue;
        };
        if let Some(key) = previous.as_ref() {
            charge_callback(key, fuel)?;
        }
        if *previous == code {
            continue;
        }
        let closures = |key: &Option<CallbackKey>| match key {
            Some(CallbackKey::Closure(id)) => Some(vec![*id]),
            Some(CallbackKey::ClosureAlternatives(ids)) => Some(ids.clone()),
            _ => None,
        };
        if let (Some(mut left), Some(right)) = (closures(previous), closures(&code)) {
            for id in right {
                charge(fuel, left.len().saturating_add(1))?;
                if !left.contains(&id) {
                    left.push(id);
                }
            }
            *previous = Some(CallbackKey::ClosureAlternatives(left));
            continue;
        }
        let names = |key: &Option<CallbackKey>| match key {
            Some(CallbackKey::Named(key)) => Some(vec![key.clone()]),
            Some(CallbackKey::Alternatives(keys)) => Some(keys.clone()),
            _ => None,
        };
        *previous = match (names(previous), names(&code)) {
            (Some(mut left), Some(right)) => {
                for key in right {
                    // Bound each structural comparison before deduplication.
                    for existing in &left {
                        charge(fuel, 1)?;
                        for argument in existing.arguments.iter() {
                            bound_type(argument, fuel)?;
                        }
                    }
                    if !left.contains(&key) {
                        left.push(key);
                    }
                }
                Some(CallbackKey::Alternatives(left))
            }
            _ => None,
        };
    }
    Ok(())
}

fn merge_places(target: &mut DescriptorPlaces, source: DescriptorPlaces) {
    for (path, targets) in source {
        match target.entry(path) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(targets);
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                match (entry.get_mut(), targets) {
                    (Some(previous), Some(incoming)) => previous.extend(incoming),
                    (previous, None) => *previous = None,
                    (None, Some(_)) => (),
                }
            }
        }
    }
}

fn merge(
    target: &mut Option<Regions>,
    source: Option<Regions>,
    fuel: &mut usize,
) -> Result<(), String> {
    if let Some(source) = source {
        charge_regions(&source, fuel)?;
        let target = target.get_or_insert_with(Regions::new);
        merge_known(&mut target.code, source.code, fuel)?;
        merge_places(&mut target.places, source.places);
        for (path, origins) in source.text {
            target.text.entry(path).or_default().extend(origins);
        }
    }
    Ok(())
}

fn project(regions: Regions, prefix: &[usize], fuel: &mut usize) -> Result<Regions, String> {
    charge_regions(&regions, fuel)?;
    Ok(Regions {
        places: regions
            .places
            .into_iter()
            .filter_map(|(path, value)| {
                path.strip_prefix(prefix).map(|rest| (rest.to_vec(), value))
            })
            .collect(),
        text: regions
            .text
            .into_iter()
            .filter_map(|(path, value)| {
                path.strip_prefix(prefix).map(|rest| (rest.to_vec(), value))
            })
            .collect(),
        code: regions
            .code
            .into_iter()
            .filter_map(|(path, value)| {
                path.strip_prefix(prefix).map(|rest| (rest.to_vec(), value))
            })
            .collect(),
    })
}

fn prefix(regions: Regions, prefix: &[usize], fuel: &mut usize) -> Result<Regions, String> {
    charge_regions(&regions, fuel)?;
    let mut output = Regions::new();
    for (path, value) in regions.places {
        charge(fuel, prefix.len())?;
        output
            .places
            .insert(prefix.iter().copied().chain(path).collect(), value);
    }
    for (path, value) in regions.text {
        charge(fuel, prefix.len())?;
        output
            .text
            .insert(prefix.iter().copied().chain(path).collect(), value);
    }
    for (path, value) in regions.code {
        charge(fuel, prefix.len())?;
        output
            .code
            .insert(prefix.iter().copied().chain(path).collect(), value);
    }
    Ok(output)
}

/// Derive conditional return dependencies by a bounded fixed point over concrete
/// named instances. Unsupported transfers fail closed, never become evidence.
pub fn infer_text_regions(
    package: &Package,
    declaration: DeclarationId,
    arguments: &[Term],
    fuel: &mut usize,
) -> Result<TextRegionSummary, String> {
    let declarations = crate::declarations::Metadata::new(Some(package));
    infer_text_regions_with_metadata(package, &declarations, declaration, arguments, fuel)
}

/// Package-scoped memoization for closed `infer_text_regions` summaries.
/// Failed or unconverged analyses are never cached; cache hits continue deducting
/// bounded work from `fuel` so exhausted budgets fail closed.
#[derive(Default)]
pub struct TextRegionCache {
    summaries: BTreeMap<OrderedInstanceKey, Summary>,
}

impl TextRegionCache {
    pub fn clear(&mut self) {
        self.summaries.clear();
    }

    pub fn len(&self) -> usize {
        self.summaries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.summaries.is_empty()
    }
}

enum TextRegionCacheStorage<'a> {
    Owned(TextRegionCache),
    Borrowed(&'a mut TextRegionCache),
}

impl Default for TextRegionCacheStorage<'_> {
    fn default() -> Self {
        Self::Owned(TextRegionCache::default())
    }
}

impl TextRegionCacheStorage<'_> {
    fn as_ref(&self) -> &TextRegionCache {
        match self {
            Self::Owned(cache) => cache,
            Self::Borrowed(cache) => cache,
        }
    }

    fn as_borrowed_mut(&mut self) -> Option<&mut TextRegionCache> {
        match self {
            Self::Owned(_) => None,
            Self::Borrowed(cache) => Some(cache),
        }
    }
}

pub(crate) fn infer_text_regions_with_metadata(
    package: &Package,
    declarations: &crate::declarations::Metadata<'_>,
    declaration: DeclarationId,
    arguments: &[Term],
    fuel: &mut usize,
) -> Result<TextRegionSummary, String> {
    let mut methods = MethodCallees::default();
    infer_text_regions_with_context(
        package,
        None,
        declarations,
        &mut methods,
        None,
        None,
        declaration,
        arguments,
        fuel,
    )
}

/// Explicit concrete callback bindings supplied for top-level text-region verification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextCallbackInput {
    Owned {
        parameter: usize,
        fields: Vec<usize>,
        targets: Vec<iir::InstanceKey>,
    },
    Borrowed {
        parameter: usize,
        input_fields: Vec<usize>,
        fields: Vec<usize>,
        targets: Vec<iir::InstanceKey>,
    },
}

impl fmt::Display for TextCallbackInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Owned {
                parameter,
                fields,
                targets,
            } => {
                write!(f, "(text-callback-owned (parameter {parameter}) (fields")?;
                display_owned_path(fields, f)?;
                f.write_str(") (targets")?;
                for target in targets {
                    write!(f, " {target}")?;
                }
                f.write_str("))")
            }
            Self::Borrowed {
                parameter,
                input_fields,
                fields,
                targets,
            } => {
                write!(
                    f,
                    "(text-callback-borrowed (parameter {parameter}) (input-fields"
                )?;
                display_owned_path(input_fields, f)?;
                f.write_str(") (fields")?;
                display_owned_path(fields, f)?;
                f.write_str(") (targets")?;
                for target in targets {
                    write!(f, " {target}")?;
                }
                f.write_str("))")
            }
        }
    }
}

fn callback_key_from_targets(targets: &[iir::InstanceKey]) -> Result<CallbackKey, String> {
    match targets {
        [] => Err("text region callback requires concrete named target(s)".into()),
        [single] => Ok(CallbackKey::Named(single.clone())),
        many => Ok(CallbackKey::Alternatives(many.to_vec())),
    }
}

/// Derive conditional return dependencies under explicit concrete callback targets.
pub fn infer_text_regions_with_callbacks(
    package: &Package,
    declaration: DeclarationId,
    arguments: &[Term],
    callbacks: &[TextCallbackInput],
    fuel: &mut usize,
) -> Result<TextRegionSummary, String> {
    let declarations = crate::declarations::Metadata::new(Some(package));
    let mut methods = MethodCallees::default();
    infer_text_regions_with_callbacks_and_context(
        package,
        None,
        &declarations,
        &mut methods,
        None,
        None,
        declaration,
        arguments,
        callbacks,
        fuel,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn infer_text_regions_with_context<'a>(
    package: &'a Package,
    source: Option<::materialization::DeclarationSource<'a>>,
    declarations: &crate::declarations::Metadata<'a>,
    methods: &mut MethodCallees,
    layout_cache: Option<&mut TypeLayoutCache>,
    text_cache: Option<&mut TextRegionCache>,
    declaration: DeclarationId,
    arguments: &[Term],
    fuel: &mut usize,
) -> Result<TextRegionSummary, String> {
    infer_text_regions_with_callbacks_and_context(
        package,
        source,
        declarations,
        methods,
        layout_cache,
        text_cache,
        declaration,
        arguments,
        &[],
        fuel,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn infer_text_regions_with_callbacks_and_context<'a>(
    package: &'a Package,
    source: Option<::materialization::DeclarationSource<'a>>,
    declarations: &crate::declarations::Metadata<'a>,
    methods: &mut MethodCallees,
    layout_cache: Option<&mut TypeLayoutCache>,
    text_cache: Option<&mut TextRegionCache>,
    declaration: DeclarationId,
    arguments: &[Term],
    callback_inputs: &[TextCallbackInput],
    fuel: &mut usize,
) -> Result<TextRegionSummary, String> {
    let mut callbacks = Vec::new();
    let mut borrowed_callbacks = Vec::new();
    for entry in callback_inputs {
        match entry {
            TextCallbackInput::Owned {
                parameter,
                fields,
                targets,
            } => callbacks.push((
                *parameter,
                fields.clone(),
                callback_key_from_targets(targets)?,
            )),
            TextCallbackInput::Borrowed {
                parameter,
                input_fields,
                fields,
                targets,
            } => borrowed_callbacks.push((
                *parameter,
                input_fields.clone(),
                fields.clone(),
                callback_key_from_targets(targets)?,
            )),
        }
    }
    Query {
        package,
        source,
        declarations,
        cached_domains: None,
        fuel,
        current: None,
        local_callables: BTreeMap::new(),
        closed_closures: vec![],
        storage_entries: vec![],
        storage_index: BTreeMap::new(),
        storage_pending: VecDeque::new(),
        storage_scheduled: BTreeSet::new(),
        storage_dependents: BTreeMap::new(),
        storage_cache_dependents: BTreeMap::new(),
        storage_outer_dependents: BTreeMap::new(),
        storage_outer_dependents_set: BTreeMap::new(),
        storage_current: None,
        solving_storage: false,
        next_storage_frame: 0,
        readonly_storage: None,
        changed_callables: BTreeSet::new(),
        pending: VecDeque::new(),
        scheduled: BTreeSet::new(),
        dependents: BTreeMap::new(),
        dependents_set: BTreeMap::new(),
        reads: vec![],
        loops: vec![],
        exits: vec![],
        cache: BTreeMap::new(),
        methods,
        layout_cache: layout_cache.map_or_else(Default::default, TypeLayoutCacheStorage::Borrowed),
        external_cache: text_cache.map_or_else(Default::default, TextRegionCacheStorage::Borrowed),
    }
    .solve_with_callbacks_and_borrowed(declaration, arguments, &callbacks, &borrowed_callbacks)
    .map(|summary| TextRegionSummary {
        declaration: summary.declaration,
        arguments: summary.arguments,
        reads: summary.reads,
        returned: summary.returned.map(|value| value.text),
    })
}

pub(crate) fn verify_handler_arm_text_regions(
    package: &Package,
    handler: &nir::program::EffectHandler,
    arm: &nir::program::MatchArm,
    fuel: &mut usize,
) -> Result<(), String> {
    let declarations = crate::declarations::Metadata::new(Some(package));
    let mut methods = MethodCallees::default();
    let mut query = Query {
        package,
        source: None,
        declarations: &declarations,
        cached_domains: None,
        fuel,
        current: None,
        local_callables: BTreeMap::new(),
        closed_closures: vec![],
        storage_entries: vec![],
        storage_index: BTreeMap::new(),
        storage_pending: VecDeque::new(),
        storage_scheduled: BTreeSet::new(),
        storage_dependents: BTreeMap::new(),
        storage_cache_dependents: BTreeMap::new(),
        storage_outer_dependents: BTreeMap::new(),
        storage_outer_dependents_set: BTreeMap::new(),
        storage_current: None,
        solving_storage: false,
        next_storage_frame: 0,
        readonly_storage: None,
        changed_callables: BTreeSet::new(),
        pending: VecDeque::new(),
        scheduled: BTreeSet::new(),
        dependents: BTreeMap::new(),
        dependents_set: BTreeMap::new(),
        reads: vec![],
        loops: vec![],
        exits: vec![],
        cache: BTreeMap::new(),
        methods: &mut methods,
        layout_cache: TypeLayoutCacheStorage::default(),
        external_cache: TextRegionCacheStorage::default(),
    };
    let Term::Function { parameters, result } = handler.signature.callable_signature() else {
        return Err("text effect handler requires a callable signature".into());
    };
    for parameter in parameters {
        if query.type_may_hold_text_or_callable(&parameter.ty)? {
            return Err("text effect handler cannot transmit text or callables".into());
        }
    }
    if query.type_may_hold_text_or_callable(result)? {
        return Err("text effect handler cannot return text or callables".into());
    }
    let subject_ty = Term::Tuple(parameters.iter().map(|p| p.ty.clone()).collect());
    let mut scope = Locals::new();
    let mut pattern = &arm.pattern;
    let mut guards = Vec::new();
    while let Pattern::Guard {
        pattern: inner,
        condition,
    } = pattern
    {
        charge(query.fuel, 1)?;
        guards.push(condition.as_ref());
        pattern = inner;
    }
    guards.reverse();
    if let Some(guard) = &arm.guard {
        guards.push(guard);
    }
    query.reads.push(BTreeSet::new());
    let checked = (|| {
        query.bind_pattern(pattern, &subject_ty, &Regions::new(), None, &mut scope, 0)?;
        for guard in guards {
            query.expr(guard, &mut scope, &mut None, 0)?;
        }
        query.expr(&arm.body, &mut scope, &mut None, 0)?;
        query.run_pending()
    })();
    query.reads.pop();
    checked
}

struct Query<'a, 'd, 'm, 'f, 'l, 'c> {
    package: &'a Package,
    source: Option<::materialization::DeclarationSource<'a>>,
    declarations: &'d crate::declarations::Metadata<'a>,
    cached_domains: Option<std::sync::Arc<BTreeMap<DeclarationId, Term>>>,
    fuel: &'f mut usize,
    current: Option<SummaryKey>,
    local_callables: BTreeMap<LocalId, CodeValue>,
    closed_closures: Vec<ClosedClosure>,
    storage_entries: Vec<StorageEntry>,
    storage_index: BTreeMap<StorageKey, usize>,
    storage_pending: VecDeque<usize>,
    storage_scheduled: BTreeSet<usize>,
    storage_dependents: BTreeMap<usize, BTreeSet<usize>>,
    storage_cache_dependents: BTreeMap<SummaryKey, BTreeSet<usize>>,
    storage_outer_dependents: BTreeMap<usize, Vec<SummaryKey>>,
    storage_outer_dependents_set: BTreeMap<usize, BTreeSet<SummaryKey>>,
    storage_current: Option<usize>,
    solving_storage: bool,
    next_storage_frame: usize,
    readonly_storage: Option<&'static str>,
    changed_callables: BTreeSet<LocalId>,
    pending: VecDeque<SummaryKey>,
    scheduled: BTreeSet<SummaryKey>,
    dependents: BTreeMap<SummaryKey, Vec<SummaryKey>>,
    dependents_set: BTreeMap<SummaryKey, BTreeSet<SummaryKey>>,
    reads: Vec<BTreeSet<TextRegionOrigin>>,
    loops: Vec<LoopRegions>,
    exits: Vec<Option<ExitRegions>>,
    cache: BTreeMap<SummaryKey, Summary>,
    methods: &'m mut MethodCallees,
    layout_cache: TypeLayoutCacheStorage<'l>,
    external_cache: TextRegionCacheStorage<'c>,
}

impl Query<'_, '_, '_, '_, '_, '_> {
    pub(super) fn borrowed_input_layout(
        &mut self,
        ty: &Term,
    ) -> Result<Vec<BorrowedInputLeaf>, String> {
        self.layout_cache
            .as_mut()
            .borrowed_input_layout(Some(self.package), ty, self.fuel)
    }

    pub(super) fn type_may_hold_text_or_callable(&mut self, ty: &Term) -> Result<bool, String> {
        self.layout_cache
            .as_mut()
            .type_may_hold_text_or_callable(self.package, ty, self.fuel)
    }

    pub(super) fn owned_callable_paths(&mut self, ty: &Term) -> Result<Vec<Vec<usize>>, String> {
        self.layout_cache
            .as_mut()
            .owned_callable_paths(Some(self.package), ty, true, self.fuel)
    }

    pub(super) fn record_reads(
        &mut self,
        origins: impl IntoIterator<Item = TextRegionOrigin>,
    ) -> Result<(), String> {
        let frame = self
            .reads
            .last_mut()
            .ok_or("text region read frame is unavailable")?;
        frame.extend(origins);
        Ok(())
    }

    fn record_exit(&mut self, locals: &Locals) -> Result<(), String> {
        if let Some(Some(exit)) = self.exits.last_mut() {
            join_locals(&mut exit.locals, locals, &exit.bindings, self.fuel)?;
        }
        Ok(())
    }

    fn contract(&mut self, predicate: &Expr, locals: &Locals) -> Result<(), String> {
        if !nir::program::checkable_predicate(predicate, self.fuel)? {
            return Err("invalid text region program predicate".into());
        }
        let mut observation = self.locals_copy(locals)?;
        self.observe_value(predicate, &mut observation, 0)?;
        Ok(())
    }

    fn observe_value(
        &mut self,
        value: &Expr,
        locals: &mut Locals,
        depth: usize,
    ) -> Result<Regions, String> {
        // Candidate rollback cannot restore writes into an existing binding.
        // Private temporaries inside an observation block are still permitted.
        nir::visit::expression(value, self.fuel, |expr, _| {
            if matches!(&expr.kind, K::WritePointer { .. }) {
                return Err("text observation cannot write through a pointer".into());
            }
            if matches!(&expr.kind, K::Assign { place, .. } if locals.contains_key(&place.root)) {
                return Err("text observation writes an existing binding".into());
            }
            Ok(())
        })?;
        let outer_loops = std::mem::take(&mut self.loops);
        let mut returned = None;
        let previous_readonly = std::mem::replace(
            &mut self.readonly_storage,
            Some("text observation cannot write through pointers"),
        );
        let result = self.expr(value, locals, &mut returned, depth);
        self.readonly_storage = previous_readonly;
        self.loops = outer_loops;
        let result = result?;
        if returned.is_some() {
            return Err("text observation has an unsupported return".into());
        }
        result.ok_or_else(|| "text observation does not complete normally".into())
    }

    // Return success knowledge only after accounting for the checks reached in
    // source order. A later known mismatch cannot erase earlier character reads.
    fn bind_pattern(
        &mut self,
        pattern: &Pattern,
        ty: &Term,
        regions: &Regions,
        value: Option<&Expr>,
        locals: &mut Locals,
        depth: usize,
    ) -> Result<Option<bool>, String> {
        if depth >= 128 {
            return Err("text region pattern depth exhausted".into());
        }
        charge(self.fuel, 1)?;
        match pattern {
            Pattern::Wild | Pattern::Unit => Ok(Some(true)),
            Pattern::Literal(literal) => {
                if matches!(literal, nir::program::Literal::String(_)) {
                    charge_regions(regions, self.fuel)?;
                    for origins in regions.values() {
                        self.reads
                            .last_mut()
                            .unwrap()
                            .extend(origins.iter().cloned());
                    }
                }
                Ok(value.and_then(|value| known_pattern(pattern, value, 0)))
            }
            Pattern::Binding { binding, inner } => {
                let matched = self.bind_pattern(inner, ty, regions, value, locals, depth + 1)?;
                if matched != Some(false) {
                    charge_regions(regions, self.fuel)?;
                    locals.insert(*binding, regions.clone());
                }
                Ok(matched)
            }
            Pattern::Tuple(patterns)
            | Pattern::Record {
                fields: patterns, ..
            }
            | Pattern::AnonymousRecord {
                fields: patterns, ..
            } => {
                let fields = owned_fields(Some(self.package), ty, self.fuel)?;
                if patterns.len() != fields.len() {
                    return Err("text pattern field arity mismatch".into());
                }
                let values = value.and_then(|value| match &value.kind {
                    K::Tuple(fields) | K::Record { fields, .. } | K::AnonymousRecord { fields } => {
                        Some(fields.as_slice())
                    }
                    _ => None,
                });
                let mut matched = Some(true);
                for (index, (pattern, ty)) in patterns.iter().zip(fields).enumerate() {
                    charge_regions(regions, self.fuel)?;
                    let nested = project(regions.clone(), &[index], self.fuel)?;
                    match self.bind_pattern(
                        pattern,
                        &ty,
                        &nested,
                        values.and_then(|v| v.get(index)),
                        locals,
                        depth + 1,
                    )? {
                        Some(false) => return Ok(Some(false)),
                        None => matched = None,
                        Some(true) => (),
                    }
                }
                Ok(matched)
            }
            Pattern::OptionalAbsent
            | Pattern::OptionalPresent(_)
            | Pattern::Variant { .. }
            | Pattern::ErrorResult { .. } => {
                let (tag, patterns, fields) =
                    owned_variant_pattern(Some(self.package), pattern, ty, self.fuel)?;
                let constructor = match value {
                    Some(value)
                        if matches!(
                            &value.kind,
                            K::Variant { .. }
                                | K::OptionalAbsent
                                | K::OptionalPresent(_)
                                | K::ErrorConvert {
                                    conversion: nir::error_results::Conversion::Success
                                        | nir::error_results::Conversion::Failure,
                                    ..
                                }
                        ) =>
                    {
                        Some(owned_constructor(value, self.fuel)?)
                    }
                    _ => None,
                };
                if constructor
                    .as_ref()
                    .is_some_and(|(_, actual)| *actual != Some(tag))
                {
                    return Ok(Some(false));
                }
                let mut matched = constructor.as_ref().map(|_| true);
                for (index, (pattern, ty)) in patterns.iter().zip(fields).enumerate() {
                    charge_regions(regions, self.fuel)?;
                    let nested = project(regions.clone(), &[VARIANT_FIELD, tag, index], self.fuel)?;
                    let value = constructor
                        .as_ref()
                        .and_then(|(values, _)| values.get(index));
                    match self.bind_pattern(pattern, &ty, &nested, value, locals, depth + 1)? {
                        Some(false) => return Ok(Some(false)),
                        None => matched = None,
                        Some(true) => (),
                    }
                }
                Ok(matched)
            }
            Pattern::Remap { pattern, bindings } => {
                let matched = self.bind_pattern(pattern, ty, regions, value, locals, depth + 1)?;
                if matched == Some(false) {
                    return Ok(matched);
                }
                let mut remapped = Locals::new();
                for (from, to) in bindings {
                    let value = locals
                        .get(from)
                        .ok_or("text pattern remap source is unavailable")?;
                    charge_regions(value, self.fuel)?;
                    remapped.insert(*to, value.clone());
                }
                locals.extend(remapped.bindings);
                Ok(matched)
            }
            Pattern::Observe { binding, inner } => {
                charge_regions(regions, self.fuel)?;
                let saved = locals.insert(*binding, regions.clone());
                let matched = self.bind_pattern(inner, ty, regions, value, locals, depth + 1);
                locals.remove(binding);
                if let Some(saved) = saved {
                    locals.insert(*binding, saved);
                }
                matched
            }
            Pattern::Guard { pattern, condition } => {
                let matched = self.bind_pattern(pattern, ty, regions, value, locals, depth + 1)?;
                if matched == Some(false) {
                    return Ok(matched);
                }
                self.observe_value(condition, locals, depth + 1)?;
                Ok(match constant_bool(condition, 0) {
                    Some(false) => Some(false),
                    Some(true) => matched,
                    None => None,
                })
            }
            Pattern::Then {
                first,
                value: computed,
                next,
            } => {
                let first = self.bind_pattern(first, ty, regions, value, locals, depth + 1)?;
                if first == Some(false) {
                    return Ok(first);
                }
                let computed_regions = self.observe_value(computed, locals, depth + 1)?;
                let next = self.bind_pattern(
                    next,
                    &computed.ty,
                    &computed_regions,
                    Some(computed),
                    locals,
                    depth + 1,
                )?;
                Ok(match (first, next) {
                    (_, Some(false)) => Some(false),
                    (Some(true), Some(true)) => Some(true),
                    _ => None,
                })
            }
            Pattern::Or(alternatives) => {
                let input = self.locals_copy(locals)?;
                let mut successes: Option<Locals> = None;
                let mut matched = Some(false);
                for alternative in alternatives {
                    let mut candidate = self.locals_copy(&input)?;
                    let success = self.bind_pattern(
                        alternative,
                        ty,
                        regions,
                        value,
                        &mut candidate,
                        depth + 1,
                    )?;
                    if success == Some(false) {
                        continue;
                    }
                    if let Some(previous) = successes.as_mut() {
                        // Only common exported identities survive alternative
                        // joins; failed candidate-local aliases never escape.
                        previous.retain(|id, _| candidate.contains_key(id));
                        let bindings: Vec<_> = previous.keys().copied().collect();
                        join_locals(&mut successes, &candidate, &bindings, self.fuel)?;
                    } else {
                        successes = Some(candidate);
                    }
                    matched = None;
                    if success == Some(true) {
                        matched = Some(true);
                        break;
                    }
                }
                if let Some(successes) = successes {
                    *locals = successes;
                }
                Ok(matched)
            }
            _ => Err("text region type-level pattern transport is not implemented".into()),
        }
    }

    fn index_local_callables(&mut self, body: &Function) -> Result<(), String> {
        let mut definitions = BTreeMap::new();
        let mut seen: BTreeSet<_> = body.parameters.iter().copied().collect();
        let mut changed = BTreeSet::new();
        nir::visit::expressions(body, self.fuel, |expr, fuel| {
            index_code_expression(expr, fuel, &mut definitions, &mut seen, &mut changed)
        })?;
        definitions.retain(|binding, _| !changed.contains(binding));
        self.local_callables = definitions;
        self.changed_callables = changed;
        Ok(())
    }

    fn known_local_callee(&mut self, binding: LocalId) -> Result<Option<CodeValue>, String> {
        let mut binding = binding;
        loop {
            charge(self.fuel, 1)?;
            let Some(value) = self.local_callables.get(&binding) else {
                return Ok(None);
            };
            if let K::Local(next) = value.expression.kind {
                binding = next;
            } else {
                return Ok(Some(value.clone()));
            }
        }
    }

    fn callable_input(&mut self, value: &Expr) -> Result<Option<CodeValue>, String> {
        self.callable_input_at(value, 0)
    }

    fn callable_input_at(
        &mut self,
        value: &Expr,
        depth: usize,
    ) -> Result<Option<CodeValue>, String> {
        let value = match &value.kind {
            K::Local(binding) => self.known_local_callee(*binding)?,
            K::Function(_)
            | K::FunctionInstance { .. }
            | K::Closure { .. }
            | K::TraitMethod { .. } => {
                nir::visit::expression(value, self.fuel, |_, _| Ok(()))?;
                Some(CodeValue::new(value.clone()))
            }
            _ => None,
        };
        value
            .map(|value| self.close_callable(value, depth))
            .transpose()
    }

    fn close_callable(&mut self, mut value: CodeValue, depth: usize) -> Result<CodeValue, String> {
        if depth >= 128 {
            return Err("text callback environment depth exhausted".into());
        }
        charge(self.fuel, 1)?;
        if matches!(value.expression.kind, K::TraitMethod { .. }) {
            value.expression = self
                .methods
                .select_with_context(
                    self.package,
                    &value.expression,
                    self.source,
                    Some(self.declarations),
                    &mut self.cached_domains,
                    self.fuel,
                    false,
                )?
                .into();
        }
        if value.closure.is_some() {
            return Ok(value);
        }
        let K::Closure {
            parameters,
            captures,
            body,
        } = &value.expression.kind
        else {
            return Ok(value);
        };
        let before = *self.fuel;
        let mut definitions = BTreeMap::new();
        let mut changed = BTreeSet::new();
        let mut seen = parameters.iter().copied().collect();
        nir::visit::expression(body, self.fuel, |expr, fuel| {
            index_code_expression(expr, fuel, &mut definitions, &mut seen, &mut changed)
        })?;
        definitions.retain(|binding, _| !changed.contains(binding));
        for capture in captures {
            charge(self.fuel, 1)?;
            if changed.contains(&capture.binding) {
                continue;
            }
            let code = if let Some(initializer) = &capture.value {
                self.callable_input_at(initializer, depth + 1)?
            } else {
                self.known_local_callee(capture.binding)?
                    .map(|code| self.close_callable(code, depth + 1))
                    .transpose()?
            };
            if let Some(code) = code {
                definitions.insert(capture.binding, code);
            }
        }
        let cost = before.saturating_sub(*self.fuel).saturating_add(1);
        for (id, known) in self.closed_closures.iter().enumerate() {
            charge(self.fuel, known.cost.saturating_add(cost))?;
            if known.expression == value.expression
                && known.definitions == definitions
                && known.changed == changed
            {
                value.closure = Some(id);
                return Ok(value);
            }
        }
        value.closure = Some(self.closed_closures.len());
        self.closed_closures.push(ClosedClosure {
            expression: value.expression.clone(),
            definitions,
            changed,
            cost,
        });
        Ok(value)
    }

    fn code_value(&self, key: &CallbackKey, expr: &Expr) -> Result<CodeValue, String> {
        match key {
            CallbackKey::Alternatives(_) | CallbackKey::ClosureAlternatives(_) => {
                Err("multiple text callees require dispatch".into())
            }
            CallbackKey::Closure(id) => {
                let code = self
                    .closed_closures
                    .get(*id)
                    .ok_or("text callable result code is unavailable")?;
                Ok(CodeValue {
                    expression: code.expression.clone(),
                    closure: Some(*id),
                })
            }
            CallbackKey::Named(instance) => Ok(CodeValue::new(Expr {
                source: expr.source,
                ty: expr.ty.clone(),
                kind: if instance.arguments.is_empty() {
                    K::Function(instance.declaration)
                } else {
                    K::FunctionInstance {
                        declaration: instance.declaration,
                        arguments: instance.arguments.to_vec(),
                    }
                },
            })),
        }
    }

    fn result_code(&self, value: &Regions, expr: &Expr) -> Result<Option<CodeValue>, String> {
        value
            .code
            .get(&vec![])
            .and_then(Option::as_ref)
            .filter(|key| {
                !matches!(
                    key,
                    CallbackKey::Alternatives(_) | CallbackKey::ClosureAlternatives(_)
                )
            })
            .map(|key| self.code_value(key, expr))
            .transpose()
    }

    fn default_input(
        &mut self,
        value: &Expr,
        prepared: &Locals,
        prepared_code: &BTreeMap<LocalId, CodeValue>,
        caller_storage: &Locals,
        depth: usize,
    ) -> Result<(Option<Regions>, Option<CodeValue>), String> {
        let has_storage = prepared
            .values()
            .any(|value| value.places.values().any(Option::is_some));
        let mut scope = if has_storage {
            self.locals_copy(caller_storage)?
        } else {
            Locals::new()
        };
        for (binding, value) in prepared.iter() {
            charge_regions(value, self.fuel)?;
            if scope.insert(*binding, value.clone()).is_some() {
                return Err("text default storage frame overlaps its caller".into());
            }
        }
        charge(
            self.fuel,
            prepared_code.len().saturating_add(prepared.len()),
        )?;
        let mut definitions = prepared_code.clone();
        let mut seen = prepared.keys().copied().collect();
        let mut changed = BTreeSet::new();
        nir::visit::expression(value, self.fuel, |expr, fuel| {
            index_code_expression(expr, fuel, &mut definitions, &mut seen, &mut changed)
        })?;
        definitions.retain(|binding, _| !changed.contains(binding));
        let caller_code = std::mem::replace(&mut self.local_callables, definitions);
        let caller_changed = std::mem::replace(&mut self.changed_callables, changed);
        let caller_loops = std::mem::take(&mut self.loops);
        self.exits.push(None);
        let caller_readonly = std::mem::replace(
            &mut self.readonly_storage,
            Some("text default argument cannot write through pointers"),
        );
        let mut returned = None;
        let result = (|| {
            let result = self.expr(value, &mut scope, &mut returned, depth)?;
            let code = result
                .as_ref()
                .map(|result| self.result_code(result, value))
                .transpose()?
                .flatten();
            if returned.is_some() {
                return Err("text default argument cannot return from its caller".into());
            }
            if let Some(value) = &result {
                charge_regions(value, self.fuel)?;
                for (root, _) in value.places.values().flatten().flatten() {
                    if !caller_storage.contains_storage(root) {
                        return Err(
                            "text default descriptor address escapes its preparation frame".into(),
                        );
                    }
                }
                if has_storage {
                    if scope.frames != caller_storage.frames {
                        return Err("text default argument modifies caller storage".into());
                    }
                    for (binding, value) in caller_storage.iter() {
                        if scope.get(binding) != Some(value) {
                            return Err("text default argument modifies caller storage".into());
                        }
                    }
                }
            }
            Ok((result, code))
        })();
        self.readonly_storage = caller_readonly;
        self.exits.pop();
        self.loops = caller_loops;
        self.local_callables = caller_code;
        self.changed_callables = caller_changed;
        result
    }

    fn schedule(&mut self, key: SummaryKey) {
        if self.scheduled.insert(key.clone()) {
            self.pending.push_back(key);
        }
    }

    fn run_pending(&mut self) -> Result<(), String> {
        let mut deferred = Vec::<(SummaryKey, String)>::new();
        while let Some(key) = self.pending.pop_front() {
            charge(self.fuel, 1)?;
            self.scheduled.remove(&key);
            self.current = Some(key.clone());
            let result = self.analyze(&key);
            self.current = None;
            deferred.retain(|(instance, _)| instance != &key);
            let mut summary = match result {
                Ok(summary) => summary,
                Err(error) => {
                    // Observations require normal completion. A newly discovered
                    // callee can still be bottom until its own work item runs.
                    // Retry on dependency growth, but never export a failed body.
                    deferred.push((key, error));
                    continue;
                }
            };
            let previous = &self.cache[&key];
            charge(self.fuel, previous.reads.len())?;
            summary.reads.extend(previous.reads.iter().cloned());
            if let Some(regions) = &previous.returned {
                charge_regions(regions, self.fuel)?;
            }
            merge(&mut summary.returned, previous.returned.clone(), self.fuel)?;
            if &summary != previous {
                self.cache.insert(key.clone(), summary);
                if let Some(callers) = self.dependents.get(&key) {
                    charge(self.fuel, callers.len())?;
                    let callers: Vec<_> = callers.iter().cloned().collect();
                    for caller in callers {
                        self.schedule(caller);
                    }
                }
                if let Some(storage_callers) = self.storage_cache_dependents.get(&key) {
                    charge(self.fuel, storage_callers.len())?;
                    let storage_callers: Vec<_> = storage_callers.iter().copied().collect();
                    for storage_caller in storage_callers {
                        self.schedule_storage(storage_caller);
                    }
                }
            }
        }
        if let Some((_, error)) = deferred.into_iter().next() {
            return Err(error);
        }
        Ok(())
    }

    fn solve_with_callbacks_and_borrowed(
        &mut self,
        declaration: DeclarationId,
        arguments: &[Term],
        callbacks: &[(usize, Vec<usize>, CallbackKey)],
        borrowed_callbacks: &[(usize, Vec<usize>, Vec<usize>, CallbackKey)],
    ) -> Result<Summary, String> {
        if callbacks.is_empty()
            && borrowed_callbacks.is_empty()
            && let Some(summary) = self
                .external_cache
                .as_ref()
                .summaries
                .get(&OrderedInstanceKey(iir::InstanceKey {
                    declaration,
                    arguments: arguments.to_vec().into(),
                }))
        {
            charge(self.fuel, arguments.len().saturating_add(1))?;
            if let Some(returned) = &summary.returned {
                charge_regions(returned, self.fuel)?;
            }
            charge(self.fuel, summary.reads.len())?;
            return Ok(summary.clone());
        }
        self.function_with_callbacks_and_borrowed(
            declaration,
            arguments,
            callbacks,
            borrowed_callbacks,
        )?;
        self.run_pending()?;
        let result = self.function_with_callbacks_and_borrowed(
            declaration,
            arguments,
            callbacks,
            borrowed_callbacks,
        )?;
        if let Some(cache) = self.external_cache.as_borrowed_mut() {
            for (key, summary) in &self.cache {
                if key.callbacks.is_empty() && key.borrowed_callbacks.is_empty() {
                    cache
                        .summaries
                        .entry(OrderedInstanceKey(key.instance.clone()))
                        .or_insert_with(|| summary.clone());
                }
            }
        }
        Ok(result)
    }

    fn function(
        &mut self,
        declaration: DeclarationId,
        arguments: &[Term],
    ) -> Result<Summary, String> {
        self.function_with_callbacks_and_borrowed(declaration, arguments, &[], &[])
    }

    fn function_with_callbacks(
        &mut self,
        declaration: DeclarationId,
        arguments: &[Term],
        callbacks: &[(usize, Vec<usize>, CallbackKey)],
    ) -> Result<Summary, String> {
        self.function_with_callbacks_and_borrowed(declaration, arguments, callbacks, &[])
    }

    fn function_with_callbacks_and_borrowed(
        &mut self,
        declaration: DeclarationId,
        arguments: &[Term],
        callbacks: &[(usize, Vec<usize>, CallbackKey)],
        borrowed_callbacks: &[(usize, Vec<usize>, Vec<usize>, CallbackKey)],
    ) -> Result<Summary, String> {
        charge(
            self.fuel,
            callbacks
                .len()
                .saturating_add(borrowed_callbacks.len())
                .saturating_add(1),
        )?;
        for (_, path, callback) in callbacks {
            charge(self.fuel, path.len())?;
            charge_callback(callback, self.fuel)?;
        }
        for (_, input_fields, path, callback) in borrowed_callbacks {
            charge(self.fuel, input_fields.len().saturating_add(path.len()))?;
            charge_callback(callback, self.fuel)?;
        }
        if callbacks.is_empty()
            && borrowed_callbacks.is_empty()
            && let Some(summary) = self
                .external_cache
                .as_ref()
                .summaries
                .get(&OrderedInstanceKey(iir::InstanceKey {
                    declaration,
                    arguments: arguments.to_vec().into(),
                }))
        {
            charge(self.fuel, arguments.len().saturating_add(1))?;
            if let Some(returned) = &summary.returned {
                charge_regions(returned, self.fuel)?;
            }
            charge(self.fuel, summary.reads.len())?;
            return Ok(summary.clone());
        }
        for argument in arguments {
            bound_type(argument, self.fuel)?;
        }
        let key = SummaryKey {
            instance: iir::InstanceKey {
                declaration,
                arguments: arguments.to_vec().into(),
            },
            callbacks: callbacks.to_vec(),
            borrowed_callbacks: borrowed_callbacks.to_vec(),
        };
        if let Some(caller) = &self.current {
            charge(self.fuel, 1)?;
            let seen = self.dependents_set.entry(key.clone()).or_default();
            if seen.insert(caller.clone()) {
                self.dependents
                    .entry(key.clone())
                    .or_default()
                    .push(caller.clone());
            }
        }
        if let Some(storage_caller) = self.storage_current {
            charge(self.fuel, 1)?;
            self.storage_cache_dependents
                .entry(key.clone())
                .or_default()
                .insert(storage_caller);
        }
        if !self.cache.contains_key(&key) {
            // Bottom means no observed normal return yet, not static storage.
            // Every discovered instance is analyzed before a result escapes.
            self.cache.insert(
                key.clone(),
                Summary {
                    declaration,
                    arguments: arguments.to_vec(),
                    reads: BTreeSet::new(),
                    returned: None,
                },
            );
            self.schedule(key.clone());
        }
        let summary = &self.cache[&key];
        if let Some(returned) = &summary.returned {
            charge_regions(returned, self.fuel)?;
        }
        charge(self.fuel, summary.reads.len())?;
        Ok(summary.clone())
    }

    fn analyze(&mut self, key: &SummaryKey) -> Result<Summary, String> {
        let declaration = key.instance.declaration;
        let arguments = key.instance.arguments.as_ref();
        let (body, arguments) = self.package.runtime_body_checked(declaration, arguments)?;
        if body.phase != FunctionPhase::Runtime || body.type_parameters.len() != arguments.len() {
            return Err("text region inference requires a concrete runtime body".into());
        }
        let body = self
            .declarations
            .specialize_function(body, arguments, self.fuel)
            .map_err(|e| e.to_string())?;
        self.index_local_callables(&body)?;
        let Term::Function { parameters, result } = body.signature.callable_signature() else {
            return Err("text region inference requires a callable signature".into());
        };
        if parameters.len() != body.parameters.len() {
            return Err("text region parameter identity mismatch".into());
        }
        for (parameter, path, callback) in &key.callbacks {
            if !path.is_empty() {
                continue;
            }
            let binding = if *parameter < body.parameters.len() {
                body.parameters[*parameter]
            } else {
                body.env_parameters
                    .get(*parameter - body.parameters.len())
                    .ok_or("text callback parameter is missing")?
                    .binding
            };
            if !self.changed_callables.contains(&binding) {
                let code = match callback {
                    CallbackKey::Named(callback) => {
                        let kind = if callback.arguments.is_empty() {
                            K::Function(callback.declaration)
                        } else {
                            K::FunctionInstance {
                                declaration: callback.declaration,
                                arguments: callback.arguments.to_vec(),
                            }
                        };
                        CodeValue::new(Expr {
                            source: body.body.source,
                            ty: parameters[*parameter].ty.clone(),
                            kind,
                        })
                    }
                    CallbackKey::Alternatives(_) | CallbackKey::ClosureAlternatives(_) => continue,
                    CallbackKey::Closure(id) => {
                        let closed = self
                            .closed_closures
                            .get(*id)
                            .ok_or("text callback code is unavailable")?;
                        CodeValue {
                            expression: closed.expression.clone(),
                            closure: Some(*id),
                        }
                    }
                };
                self.local_callables.insert(binding, code);
            }
        }
        let mut locals = Locals::new();
        for (parameter, (id, ty)) in body.parameters.iter().zip(parameters).enumerate() {
            if ty.sort != nir::Sort::Runtime
                && matches!(
                    ty.ty,
                    Term::Primitive(Primitive::Type)
                        | Term::ClosureContextType
                        | Term::EffectRowType
                        | Term::ErrorRowType
                        | Term::ReachabilityType
                        | Term::AnyType
                )
            {
                locals.insert(*id, Regions::new());
                continue;
            }
            let mut regions = Regions::new();
            regions.code = self
                .owned_callable_paths(&ty.ty)?
                .into_iter()
                .map(|path| (path, None))
                .collect();
            for (_, path, code) in key
                .callbacks
                .iter()
                .filter(|(index, _, _)| *index == parameter)
            {
                regions.code.insert(path.clone(), Some(code.clone()));
            }
            for leaf in self.borrowed_input_layout(&ty.ty)? {
                if let BorrowedInputLeaf::Pointer(leaf) = &leaf {
                    regions.places.insert(leaf.input_fields.clone(), None);
                }
                if let BorrowedInputLeaf::Text(leaf) = leaf {
                    regions.insert(
                        leaf.input_fields.clone(),
                        BTreeSet::from([TextRegionOrigin::Parameter {
                            parameter,
                            fields: leaf.input_fields,
                        }]),
                    );
                }
            }
            let mut borrowed_frames: BTreeMap<Vec<usize>, usize> = BTreeMap::new();
            for (_, input_fields, path, code) in key
                .borrowed_callbacks
                .iter()
                .filter(|(index, _, _, _)| *index == parameter)
            {
                let frame_id = *borrowed_frames
                    .entry(input_fields.clone())
                    .or_insert_with(|| {
                        let frame_id = self.next_storage_frame;
                        self.next_storage_frame += 1;
                        locals.frames.insert(frame_id, Regions::new());
                        regions.places.insert(
                            input_fields.clone(),
                            Some(BTreeSet::from([(DescriptorRoot::Frame(frame_id), vec![])])),
                        );
                        frame_id
                    });
                if let Some(frame) = locals.frames.get_mut(&frame_id) {
                    frame.code.insert(path.clone(), Some(code.clone()));
                }
            }
            locals.insert(*id, regions);
        }
        for (offset, env) in body.env_parameters.iter().enumerate() {
            let parameter = parameters.len() + offset;
            let mut regions = Regions::new();
            regions.code = self
                .owned_callable_paths(&env.ty)?
                .into_iter()
                .map(|path| (path, None))
                .collect();
            for (_, path, code) in key
                .callbacks
                .iter()
                .filter(|(index, _, _)| *index == parameter)
            {
                regions.code.insert(path.clone(), Some(code.clone()));
            }
            for leaf in self.borrowed_input_layout(&env.ty)? {
                if let BorrowedInputLeaf::Pointer(leaf) = &leaf {
                    regions.places.insert(leaf.input_fields.clone(), None);
                }
                if let BorrowedInputLeaf::Text(leaf) = leaf {
                    regions.insert(
                        leaf.input_fields.clone(),
                        BTreeSet::from([TextRegionOrigin::Parameter {
                            parameter,
                            fields: leaf.input_fields,
                        }]),
                    );
                }
            }
            if locals.insert(env.binding, regions).is_some() {
                return Err("text region env parameter identity mismatch".into());
            }
        }
        for snapshot in &body.entry_snapshots {
            let value = locals
                .get(&snapshot.parameter)
                .ok_or("text entry snapshot parameter is unavailable")?;
            charge_regions(value, self.fuel)?;
            let value = value.clone();
            if locals.insert(snapshot.binding, value).is_some() {
                return Err("text entry snapshot binding is not fresh".into());
            }
        }
        self.reads.push(BTreeSet::new());
        self.exits
            .push((!body.postconditions.is_empty()).then(|| ExitRegions {
                bindings: locals.keys().copied().collect(),
                locals: None,
            }));
        let mut returned = None;
        let caller_loops = std::mem::take(&mut self.loops);
        let checked = (|| {
            for predicate in &body.preconditions {
                self.contract(predicate, &locals)?;
            }
            let completed = self.expr(&body.body, &mut locals, &mut returned, 0)?;
            if completed.is_some() {
                self.record_exit(&locals)?;
            }
            merge(&mut returned, completed, self.fuel)?;
            if let Some(origins) = &returned {
                if let Some(exit) = self.exits.last_mut().unwrap().as_mut() {
                    let mut environment = exit
                        .locals
                        .take()
                        .ok_or("text postcondition exit environment is unavailable")?;
                    if let Some(binding) = body.result_binding {
                        charge_regions(origins, self.fuel)?;
                        environment.insert(binding, origins.clone());
                    }
                    for predicate in &body.postconditions {
                        self.contract(predicate, &environment)?;
                    }
                }
            }
            Ok::<(), String>(())
        })();
        self.loops = caller_loops;
        self.exits.pop();
        let reads = self.reads.pop().unwrap();
        checked?;
        if let Some(returned) = &returned {
            if returned.places.values().any(Option::is_some) {
                return Err("text summary cannot export local descriptor addresses".into());
            }
            let mut completion = result.as_ref();
            while let Term::Computation { result: inner, .. } = completion {
                completion = inner.as_ref();
            }
            for leaf in self.borrowed_input_layout(completion)? {
                if let BorrowedInputLeaf::Text(leaf) = leaf {
                    if leaf.guards.is_empty()
                        && returned
                            .get(&leaf.input_fields)
                            .is_none_or(BTreeSet::is_empty)
                    {
                        return Err("returned text view has no backing-region origin".into());
                    }
                }
            }
        }
        let summary = Summary {
            declaration,
            arguments: key.instance.arguments.to_vec(),
            reads,
            returned,
        };
        Ok(summary)
    }

    // Canonical frame numbers describe input alias topology, independent of
    // source bindings and the invocation-local IDs used by closure frames.
    fn call_with_storage(
        &mut self,
        declaration: DeclarationId,
        arguments: &[Term],
        mut actual: BTreeMap<usize, Regions>,
        locals: &mut Locals,
        _depth: usize,
    ) -> Result<Option<Regions>, String> {
        let before = *self.fuel;
        let mut mapping = BTreeMap::new();
        let mut restore = BTreeMap::new();
        let mut storage = BTreeMap::new();
        let mut pending = VecDeque::new();
        for value in actual.values() {
            charge_regions(value, self.fuel)?;
            pending.extend(
                value
                    .places
                    .values()
                    .flatten()
                    .flatten()
                    .map(|(root, _)| *root),
            );
        }
        while let Some(root) = pending.pop_front() {
            charge(self.fuel, 1)?;
            if mapping.contains_key(&root) {
                continue;
            }
            let value = locals
                .get_storage(&root)
                .ok_or("text call descriptor storage is no longer live")?;
            charge_regions(value, self.fuel)?;
            let id = storage.len();
            let canonical = DescriptorRoot::Frame(id);
            mapping.insert(root, canonical);
            restore.insert(canonical, root);
            pending.extend(
                value
                    .places
                    .values()
                    .flatten()
                    .flatten()
                    .map(|(root, _)| *root),
            );
            storage.insert(id, value.clone());
        }
        for value in actual.values_mut().chain(storage.values_mut()) {
            remap_descriptor_roots(value, &mapping);
        }
        for argument in arguments {
            bound_type(argument, self.fuel)?;
        }
        let key = StorageKey {
            declaration,
            arguments: arguments.to_vec(),
            actual,
            storage,
            readonly: self.readonly_storage,
        };
        let cost = before.saturating_sub(*self.fuel).saturating_add(1);
        charge(self.fuel, cost)?;
        let id = if let Some(&id) = self.storage_index.get(&key) {
            id
        } else {
            let id = self.storage_entries.len();
            self.storage_index.insert(key.clone(), id);
            self.storage_entries.push(StorageEntry {
                key,
                summary: StorageSummary::default(),
                cost,
            });
            self.schedule_storage(id);
            id
        };
        if let Some(caller) = self.storage_current {
            charge(self.fuel, 1)?;
            self.storage_dependents
                .entry(id)
                .or_default()
                .insert(caller);
        }
        if let Some(outer) = &self.current {
            charge(self.fuel, 1)?;
            let seen = self.storage_outer_dependents_set.entry(id).or_default();
            if seen.insert(outer.clone()) {
                self.storage_outer_dependents
                    .entry(id)
                    .or_default()
                    .push(outer.clone());
            }
        }
        if !self.solving_storage && !self.storage_pending.is_empty() {
            self.solving_storage = true;
            let result = self.solve_storage();
            self.solving_storage = false;
            result?;
        }
        let summary = &self.storage_entries[id].summary;
        charge(self.fuel, summary.reads.len())?;
        self.reads
            .last_mut()
            .ok_or("text region read frame is unavailable")?
            .extend(summary.reads.iter().cloned());
        let Some(value) = &summary.returned else {
            return Ok(None);
        };
        charge_regions(value, self.fuel)?;
        let mut value = value.clone();
        remap_descriptor_roots(&mut value, &restore);
        let storage = summary
            .storage
            .as_ref()
            .ok_or("text storage summary lacks normal exit state")?;
        for (canonical, root) in &restore {
            let DescriptorRoot::Frame(id) = canonical else {
                unreachable!()
            };
            let output = storage
                .get(id)
                .ok_or("text storage summary lost an input slot")?;
            charge_regions(output, self.fuel)?;
            let mut output = output.clone();
            remap_descriptor_roots(&mut output, &restore);
            *locals
                .get_storage_mut(root)
                .ok_or("text call lost caller storage")? = output;
        }
        Ok(Some(value))
    }

    fn schedule_storage(&mut self, id: usize) {
        if self.storage_scheduled.insert(id) {
            self.storage_pending.push_back(id);
        }
    }

    fn solve_storage(&mut self) -> Result<(), String> {
        let mut deferred = Vec::<(usize, String)>::new();
        while let Some(id) = self.storage_pending.pop_front() {
            self.storage_scheduled.remove(&id);
            charge(self.fuel, self.storage_entries[id].cost)?;
            let key = self.storage_entries[id].key.clone();
            let previous_current = self.storage_current.replace(id);
            let result = self.analyze_storage(&key);
            self.storage_current = previous_current;
            deferred.retain(|(entry_id, _)| *entry_id != id);
            let mut next = match result {
                Ok(summary) => summary,
                Err(error) => {
                    deferred.push((id, error));
                    continue;
                }
            };
            let previous = &self.storage_entries[id].summary;
            charge(self.fuel, previous.reads.len())?;
            next.reads.extend(previous.reads.iter().cloned());
            if let Some(value) = &previous.returned {
                charge_regions(value, self.fuel)?;
                merge(&mut next.returned, Some(value.clone()), self.fuel)?;
            }
            if let Some(storage) = &previous.storage {
                let output = next.storage.get_or_insert_with(BTreeMap::new);
                for (slot, value) in storage {
                    charge_regions(value, self.fuel)?;
                    let mut joined = output.remove(slot);
                    merge(&mut joined, Some(value.clone()), self.fuel)?;
                    output.insert(
                        *slot,
                        joined.ok_or("text storage slot join produced no state")?,
                    );
                }
            }
            if next != *previous {
                self.storage_entries[id].summary = next;
                if let Some(callers) = self.storage_dependents.get(&id) {
                    charge(self.fuel, callers.len())?;
                    let callers: Vec<_> = callers.iter().copied().collect();
                    for caller in callers {
                        self.schedule_storage(caller);
                    }
                }
                if let Some(outers) = self.storage_outer_dependents.get(&id) {
                    charge(self.fuel, outers.len())?;
                    let outers: Vec<_> = outers.iter().cloned().collect();
                    for outer in outers {
                        if self.current.as_ref() != Some(&outer) {
                            self.schedule(outer);
                        }
                    }
                }
            }
        }
        if let Some((_, error)) = deferred.first() {
            let error = error.clone();
            for (failed_id, _) in deferred {
                self.schedule_storage(failed_id);
            }
            return Err(error);
        }
        Ok(())
    }

    fn analyze_storage(&mut self, key: &StorageKey) -> Result<StorageSummary, String> {
        let (body, arguments) = self
            .package
            .runtime_body_checked(key.declaration, &key.arguments)?;
        if body.phase != FunctionPhase::Runtime || body.type_parameters.len() != arguments.len() {
            return Err("text descriptor call requires a concrete runtime body".into());
        }
        let body = self
            .declarations
            .specialize_function(body, arguments, self.fuel)
            .map_err(|e| e.to_string())?;
        let mut scope = Locals {
            bindings: BTreeMap::new(),
            frames: key.storage.clone(),
        };
        // Fresh closure frames must never collide with canonical input roots.
        self.next_storage_frame = self.next_storage_frame.max(scope.frames.len());
        for (parameter, value) in &key.actual {
            let binding = if *parameter < body.parameters.len() {
                body.parameters[*parameter]
            } else {
                body.env_parameters
                    .get(*parameter - body.parameters.len())
                    .ok_or("text descriptor parameter is missing")?
                    .binding
            };
            charge_regions(value, self.fuel)?;
            if scope.insert(binding, value.clone()).is_some() {
                return Err("text descriptor parameter identity mismatch".into());
            }
        }
        for snapshot in &body.entry_snapshots {
            let value = scope
                .get(&snapshot.parameter)
                .ok_or("text descriptor snapshot parameter is missing")?;
            charge_regions(value, self.fuel)?;
            let value = value.clone();
            if scope.insert(snapshot.binding, value).is_some() {
                return Err("text descriptor snapshot binding is not fresh".into());
            }
        }
        let definitions = std::mem::take(&mut self.local_callables);
        let changed = std::mem::take(&mut self.changed_callables);
        let loops = std::mem::take(&mut self.loops);
        let readonly = std::mem::replace(&mut self.readonly_storage, key.readonly);
        self.reads.push(BTreeSet::new());
        self.exits.push(Some(ExitRegions {
            bindings: scope.keys().copied().collect(),
            locals: None,
        }));
        let checked = (|| {
            self.index_local_callables(&body)?;
            for predicate in &body.preconditions {
                self.contract(predicate, &scope)?;
            }
            let mut returned = None;
            let completed = self.expr(&body.body, &mut scope, &mut returned, 0)?;
            if completed.is_some() {
                self.record_exit(&scope)?;
            }
            merge(&mut returned, completed, self.fuel)?;
            let mut storage = None;
            if let Some(value) = &returned {
                let mut exit = self
                    .exits
                    .last_mut()
                    .unwrap()
                    .as_mut()
                    .unwrap()
                    .locals
                    .take()
                    .ok_or("text descriptor call has no normal exit state")?;
                for value in std::iter::once(value).chain(exit.frames.values()) {
                    charge_regions(value, self.fuel)?;
                    for (root, _) in value.places.values().flatten().flatten() {
                        if !matches!(root, DescriptorRoot::Frame(id) if key.storage.contains_key(id))
                        {
                            return Err("text descriptor address escapes its callee frame".into());
                        }
                    }
                }
                if let Some(binding) = body.result_binding {
                    charge_regions(value, self.fuel)?;
                    exit.insert(binding, value.clone());
                }
                for predicate in &body.postconditions {
                    self.contract(predicate, &exit)?;
                }
                storage = Some(exit.frames);
            }
            Ok(StorageSummary {
                returned,
                storage,
                reads: BTreeSet::new(),
            })
        })();
        self.exits.pop();
        let reads = self.reads.pop().unwrap();
        self.readonly_storage = readonly;
        self.local_callables = definitions;
        self.changed_callables = changed;
        self.loops = loops;
        checked.map(|mut summary| {
            summary.reads = reads;
            summary
        })
    }

    fn translate(
        &mut self,
        origins: &BTreeSet<TextRegionOrigin>,
        actual: &BTreeMap<usize, Regions>,
    ) -> Result<BTreeSet<TextRegionOrigin>, String> {
        let mut translated = BTreeSet::new();
        for origin in origins {
            charge(self.fuel, 1)?;
            match origin {
                TextRegionOrigin::Static => {
                    translated.insert(TextRegionOrigin::Static);
                }
                TextRegionOrigin::Parameter { parameter, fields } => {
                    charge(self.fuel, fields.len())?;
                    let value = actual
                        .get(parameter)
                        .ok_or("missing text region call argument")?;
                    if let Some(origins) = value.get(fields) {
                        charge(self.fuel, origins.len())?;
                        translated.extend(origins.iter().cloned());
                    }
                }
            }
        }
        Ok(translated)
    }

    fn locals_copy(&mut self, locals: &Locals) -> Result<Locals, String> {
        charge(self.fuel, locals.len().saturating_add(locals.frames.len()))?;
        for regions in locals.values().chain(locals.frames.values()) {
            charge_regions(regions, self.fuel)?;
        }
        Ok(locals.clone())
    }

    fn invoke_named(
        &mut self,
        declaration: DeclarationId,
        types: &[Term],
        actual: BTreeMap<usize, Regions>,
        callbacks: &[(usize, Vec<usize>, CallbackKey)],
        locals: &mut Locals,
        depth: usize,
    ) -> Result<Option<Regions>, String> {
        let mut output = Some(Regions::new());
        if actual
            .values()
            .any(|value| value.places.values().any(Option::is_some))
        {
            return self.call_with_storage(declaration, types, actual, locals, depth);
        }
        let summary = self.function_with_callbacks(declaration, types, callbacks)?;
        let reads = self.translate(&summary.reads, &actual)?;
        self.reads.last_mut().unwrap().extend(reads);
        let Some(result) = summary.returned else {
            return Ok(None);
        };
        output.as_mut().unwrap().places = result.places;
        output.as_mut().unwrap().code = result.code;
        for (path, origins) in result.text {
            let translated = self.translate(&origins, &actual)?;
            output.as_mut().unwrap().insert(path, translated);
        }
        Ok(output)
    }

    fn named_call(
        &mut self,
        candidates: &[iir::InstanceKey],
        arguments: &[nir::program::CallArgument],
        env_arguments: &[Expr],
        parameter_bindings: &[LocalId],
        locals: &mut Locals,
        returned: &mut Option<Regions>,
        depth: usize,
    ) -> Result<Option<Regions>, String> {
        if candidates.is_empty() {
            return Err("text callable candidates are empty".into());
        }
        let first = &candidates[0];
        let has_defaults = arguments.iter().any(|argument| argument.is_default);
        charge(
            self.fuel,
            arguments
                .len()
                .saturating_add(env_arguments.len())
                .saturating_add(parameter_bindings.len()),
        )?;
        let ordinary_count = self
            .package
            .runtime_body_checked(first.declaration, &first.arguments)?
            .0
            .parameters
            .len();
        for candidate in candidates {
            charge(self.fuel, 1)?;
            let (body, _) = self
                .package
                .runtime_body_checked(candidate.declaration, &candidate.arguments)?;
            validate_argument_plan(arguments, parameter_bindings, body.parameters.len())?;
            if body.parameters.len() != ordinary_count
                || body.env_parameters.len() != env_arguments.len()
            {
                return Err("text region runtime environment arity mismatch".into());
            }
            if has_defaults && parameter_bindings != &body.parameters {
                return Err("text default parameter frame does not match callee".into());
            }
        }
        let mut actual = BTreeMap::new();
        let mut prepared = Locals::new();
        let mut prepared_code = BTreeMap::new();
        let mut callbacks = Vec::new();
        for argument in arguments {
            let (value, callback) = if argument.is_default {
                self.default_input(&argument.value, &prepared, &prepared_code, locals, depth)?
            } else {
                let value = self.expr(&argument.value, locals, returned, depth)?;
                let code = value
                    .as_ref()
                    .map(|value| self.result_code(value, &argument.value))
                    .transpose()?
                    .flatten();
                (value, code)
            };
            let Some(value) = value else {
                return Ok(None);
            };
            if has_defaults {
                let binding = parameter_bindings[argument.parameter];
                charge_regions(&value, self.fuel)?;
                prepared.insert(binding, value.clone());
                if let Some(code) = &callback {
                    prepared_code.insert(binding, code.clone());
                }
            }
            charge_regions(&value, self.fuel)?;
            for (path, code) in &value.code {
                if let Some(code) = code {
                    callbacks.push((argument.parameter, path.clone(), code.clone()));
                }
            }
            if actual.insert(argument.parameter, value).is_some() {
                return Err("duplicate text region call argument".into());
            }
        }
        for (offset, env_arg) in env_arguments.iter().enumerate() {
            let Some(value) = self.expr(env_arg, locals, returned, depth)? else {
                return Ok(None);
            };
            let callback = self.result_code(&value, env_arg)?;
            if let Some(code) = &callback {
                let _ = code;
            }
            charge_regions(&value, self.fuel)?;
            let parameter = ordinary_count + offset;
            for (path, code) in &value.code {
                if let Some(code) = code {
                    callbacks.push((parameter, path.clone(), code.clone()));
                }
            }
            if actual.insert(parameter, value).is_some() {
                return Err("duplicate text region env call argument".into());
            }
        }

        callbacks.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
        if candidates.len() == 1 {
            return self.invoke_named(
                first.declaration,
                &first.arguments,
                actual,
                &callbacks,
                locals,
                depth,
            );
        }
        let bindings = locals.keys().copied().collect::<Vec<_>>();
        let mut exits = None;
        let mut output = None;
        for candidate in candidates {
            charge(self.fuel, 1)?;
            for value in actual.values() {
                charge_regions(value, self.fuel)?;
            }
            let mut scope = self.locals_copy(locals)?;
            let result = self.invoke_named(
                candidate.declaration,
                &candidate.arguments,
                actual.clone(),
                &callbacks,
                &mut scope,
                depth,
            )?;
            if result.is_some() {
                join_locals(&mut exits, &scope, &bindings, self.fuel)?;
            }
            merge(&mut output, result, self.fuel)?;
        }
        if let Some(exits) = exits {
            *locals = exits;
        }
        Ok(output)
    }

    fn invoke_closed_callback(
        &mut self,
        resolved: &CodeValue,
        environment: &Regions,
        actual: &[(usize, Regions, Option<CodeValue>)],
        actual_envs: &[(Regions, Option<CodeValue>)],
        locals: &mut Locals,
        depth: usize,
    ) -> Result<Option<Regions>, String> {
        let K::Closure {
            parameters,
            captures,
            body,
        } = &resolved.expression.kind
        else {
            return Err("text closure alternative has no closure body".into());
        };
        let closure_envs = nir::program::implicit_runtime_env_parameters(
            &self.package.runtime_domains,
            &self.package.global_runtime_deliveries,
            std::slice::from_ref(&resolved.expression.ty),
            &[],
        )
        .map_err(|reason| reason.to_string())?;
        if closure_envs.len() != actual_envs.len() {
            return Err("text region closure runtime environment arity mismatch".into());
        }
        charge(
            self.fuel,
            parameters
                .len()
                .saturating_add(captures.len())
                .saturating_add(closure_envs.len()),
        )?;
        let mut scope = Locals::new();
        for (index, capture) in captures.iter().enumerate() {
            charge_regions(environment, self.fuel)?;
            let value = project(environment.clone(), &[index], self.fuel)?;
            if scope.insert(capture.binding, value).is_some() {
                return Err("duplicate text closure capture binding".into());
            }
        }
        let mut input_code = Vec::new();
        for (parameter, value, callable) in actual {
            charge_regions(value, self.fuel)?;
            if scope
                .insert(parameters[*parameter], value.clone())
                .is_some()
            {
                return Err("text closure parameter overlaps another binding".into());
            }
            input_code.push((parameters[*parameter], callable.clone()));
        }
        for (env, (value, callable)) in closure_envs.iter().zip(actual_envs) {
            charge_regions(value, self.fuel)?;
            if scope.insert(env.binding, value.clone()).is_some() {
                return Err("text closure runtime environment overlaps another binding".into());
            }
            input_code.push((env.binding, callable.clone()));
        }
        let mut storage_map = BTreeMap::new();
        let mut storage_restore = BTreeMap::new();
        // Start at pointer-bearing inputs, then follow descriptor
        // addresses transitively. Unrelated caller locals are not
        // part of the closure's storage footprint.
        let mut reachable = VecDeque::new();
        for value in scope.values() {
            charge_regions(value, self.fuel)?;
            reachable.extend(
                value
                    .places
                    .values()
                    .flatten()
                    .flatten()
                    .map(|(root, _)| *root),
            );
        }
        while let Some(source) = reachable.pop_front() {
            charge(self.fuel, 1)?;
            if storage_map.contains_key(&source) {
                continue;
            }
            let value = locals
                .get_storage(&source)
                .ok_or("text closure descriptor storage is no longer live")?;
            charge_regions(value, self.fuel)?;
            let id = self.next_storage_frame;
            self.next_storage_frame = id
                .checked_add(1)
                .ok_or("text storage frame identity exhausted")?;
            let target = DescriptorRoot::Frame(id);
            storage_map.insert(source, target);
            storage_restore.insert(target, source);
            reachable.extend(
                value
                    .places
                    .values()
                    .flatten()
                    .flatten()
                    .map(|(root, _)| *root),
            );
            scope.frames.insert(id, value.clone());
        }
        for value in scope.bindings.values_mut().chain(scope.frames.values_mut()) {
            charge_regions(value, self.fuel)?;
            remap_descriptor_roots(value, &storage_map);
        }
        let closed_context = resolved.closure;
        let previous_context = if let Some(id) = closed_context {
            let context = &self.closed_closures[id];
            charge(self.fuel, context.cost)?;
            Some((
                std::mem::replace(&mut self.local_callables, context.definitions.clone()),
                std::mem::replace(&mut self.changed_callables, context.changed.clone()),
            ))
        } else {
            None
        };
        // Install invocation-local code only after every input has
        // been evaluated in the caller's environment.
        for (binding, callable) in &mut input_code {
            let old = if self.changed_callables.contains(binding) {
                self.local_callables.remove(binding)
            } else if let Some(code) = callable.take() {
                self.local_callables.insert(*binding, code)
            } else {
                self.local_callables.remove(binding)
            };
            *callable = old;
        }
        let outer_loops = std::mem::take(&mut self.loops);
        self.exits.push(Some(ExitRegions {
            bindings: scope.keys().copied().collect(),
            locals: None,
        }));
        let mut closure_returns = None;
        let completed = self.expr(body, &mut scope, &mut closure_returns, depth);
        let completed = completed.and_then(|value| {
            if value.is_some() {
                self.record_exit(&scope)?;
            }
            Ok(value)
        });
        let exit = self.exits.pop().unwrap().unwrap().locals;
        self.loops = outer_loops;
        for (binding, previous) in input_code {
            self.local_callables.remove(&binding);
            if let Some(previous) = previous {
                self.local_callables.insert(binding, previous);
            }
        }
        if let Some((definitions, changed)) = previous_context {
            self.local_callables = definitions;
            self.changed_callables = changed;
        }
        merge(&mut closure_returns, completed?, self.fuel)?;
        if let Some(value) = &mut closure_returns {
            let mut exit = exit.ok_or("text closure has no normal exit storage")?;
            for value in std::iter::once(&mut *value).chain(exit.frames.values_mut()) {
                charge_regions(value, self.fuel)?;
                for (root, _) in value.places.values().flatten().flatten() {
                    if !storage_restore.contains_key(root) {
                        return Err("text descriptor address escapes its closure frame".into());
                    }
                }
                remap_descriptor_roots(value, &storage_restore);
            }
            for (target, source) in &storage_restore {
                let DescriptorRoot::Frame(id) = target else {
                    unreachable!()
                };
                let value = exit
                    .frames
                    .remove(id)
                    .ok_or("text closure lost caller storage")?;
                *locals
                    .get_storage_mut(source)
                    .ok_or("text closure caller storage is no longer live")? = value;
            }
        }
        Ok(closure_returns)
    }

    fn closed_callback_choices(
        &mut self,
        candidates: &[usize],
        environment: &Regions,
        callee: &Expr,
        arguments: &[nir::program::CallArgument],
        env_arguments: &[Expr],
        parameter_bindings: &[LocalId],
        locals: &mut Locals,
        returned: &mut Option<Regions>,
        depth: usize,
    ) -> Result<Option<Regions>, String> {
        if candidates.is_empty() {
            return Err("text closure candidates are empty".into());
        }
        if arguments.iter().any(|argument| argument.is_default) {
            return Err("text closure default arguments are not supported".into());
        }
        for id in candidates {
            charge(self.fuel, 1)?;
            let code = self.code_value(&CallbackKey::Closure(*id), callee)?;
            let K::Closure { parameters, .. } = &code.expression.kind else {
                return Err("text closure candidate is invalid".into());
            };
            validate_argument_plan(arguments, parameter_bindings, parameters.len())?;
            let closure_envs = nir::program::implicit_runtime_env_parameters(
                &self.package.runtime_domains,
                &self.package.global_runtime_deliveries,
                std::slice::from_ref(&code.expression.ty),
                &[],
            )
            .map_err(|reason| reason.to_string())?;
            if closure_envs.len() != env_arguments.len() {
                return Err("text region closure runtime environment arity mismatch".into());
            }
        }
        let mut actual = Vec::new();
        for argument in arguments {
            let Some(value) = self.expr(&argument.value, locals, returned, depth)? else {
                return Ok(None);
            };
            let code = self.result_code(&value, &argument.value)?;
            actual.push((argument.parameter, value, code));
        }
        let mut actual_envs = Vec::with_capacity(env_arguments.len());
        for env_arg in env_arguments {
            let Some(value) = self.expr(env_arg, locals, returned, depth)? else {
                return Ok(None);
            };
            let code = self.result_code(&value, env_arg)?;
            actual_envs.push((value, code));
        }
        if candidates.len() == 1 {
            let resolved = self.code_value(&CallbackKey::Closure(candidates[0]), callee)?;
            return self.invoke_closed_callback(
                &resolved,
                environment,
                &actual,
                &actual_envs,
                locals,
                depth,
            );
        }
        let bindings = locals.keys().copied().collect::<Vec<_>>();
        let mut exits = None;
        let mut output = None;
        for id in candidates {
            charge(self.fuel, 1)?;
            let resolved = self.code_value(&CallbackKey::Closure(*id), callee)?;
            let mut scope = self.locals_copy(locals)?;
            let result = self.invoke_closed_callback(
                &resolved,
                environment,
                &actual,
                &actual_envs,
                &mut scope,
                depth,
            )?;
            if result.is_some() {
                join_locals(&mut exits, &scope, &bindings, self.fuel)?;
            }
            merge(&mut output, result, self.fuel)?;
        }
        if let Some(exits) = exits {
            *locals = exits;
        }
        Ok(output)
    }

    fn expr(
        &mut self,
        expr: &Expr,
        locals: &mut Locals,
        returned: &mut Option<Regions>,
        depth: usize,
    ) -> Result<Option<Regions>, String> {
        if depth >= 128 {
            return Err("text region expression depth exhausted".into());
        }
        charge(self.fuel, 1)?;
        bound_type(&expr.ty, self.fuel)?;
        let next = depth + 1;
        let empty = Some(Regions::new());
        match &expr.kind {
            K::Literal(nir::program::Literal::String(_)) => Ok(Some(Regions::from(
                BTreeMap::from([(vec![], BTreeSet::from([TextRegionOrigin::Static]))]),
            ))),
            K::ComptimeValue(_) | K::AssociatedValue { .. } => {
                if !self.owned_callable_paths(&expr.ty)?.is_empty() {
                    return Err("text region comptime value cannot embed a callable".into());
                }
                if matches!(expr.ty, Term::Primitive(Primitive::Type | Primitive::Unit)) {
                    return Ok(empty);
                }
                let mut regions = Regions::new();
                for leaf in self.borrowed_input_layout(&expr.ty)? {
                    match leaf {
                        BorrowedInputLeaf::Pointer(_) => {
                            return Err("text region comptime value cannot embed a pointer".into());
                        }
                        BorrowedInputLeaf::Text(leaf) => {
                            regions.insert(
                                leaf.input_fields,
                                BTreeSet::from([TextRegionOrigin::Static]),
                            );
                        }
                    }
                }
                Ok(Some(regions))
            }
            K::Literal(_) | K::TypeValue(_) | K::Unit => Ok(empty),
            K::EffectComputation { body, .. }
            | K::EffectRun {
                computation: body, ..
            } => self.expr(body, locals, returned, next),
            K::EffectResume(value) => self.transfer_effect_resume(value, locals, returned, next),
            K::EffectRequest { arguments, .. } => {
                self.transfer_effect_request(expr, arguments, locals, returned, next)
            }
            K::EffectHandle {
                computation,
                handlers,
                ..
            } => self.transfer_effect_handle(computation, handlers, locals, returned, next),
            K::Function { .. } => self.transfer_function(expr, locals, returned, next),
            K::TraitMethod { .. } => self.transfer_trait_method(expr, locals, returned, next),
            K::FunctionInstance { .. } => {
                self.transfer_function_instance(expr, locals, returned, next)
            }
            K::Local { .. } => self.transfer_local(expr, locals, returned, next),
            K::AddressOf { .. } => self.transfer_address_of(expr, locals, returned, next),
            K::ConstAddress { .. } => Ok(empty),
            K::PointerField { .. } | K::PointerAnonymousField { .. } => {
                self.transfer_pointer_field(expr, locals, returned, next)
            }
            K::ReadPointer { .. } => self.transfer_read_pointer(expr, locals, returned, next),
            K::WritePointer { .. } => self.transfer_write_pointer(expr, locals, returned, next),
            K::SymbolName { .. } | K::EnumName { .. } => {
                self.transfer_symbol_name(expr, locals, returned, next)
            }
            K::Tuple { .. } | K::Record { .. } | K::AnonymousRecord { .. } => {
                self.transfer_tuple(expr, locals, returned, next)
            }
            K::Variant { .. } => self.transfer_variant(expr, locals, returned, next),
            K::OptionalAbsent => Ok(empty),
            K::OptionalPresent(value) => self
                .expr(value, locals, returned, next)?
                .map(|value| prefix(value, &[VARIANT_FIELD, 1, 0], self.fuel))
                .transpose(),
            K::Field { value, index, .. } | K::AnonymousField { value, index, .. } => self
                .expr(value, locals, returned, next)?
                .map(|value| project(value, &[*index], self.fuel))
                .transpose(),
            K::Let { .. } => self.transfer_let(expr, locals, returned, next),
            K::LetPattern { .. } => self.transfer_let_pattern(expr, locals, returned, next),
            K::Assign { .. } => self.transfer_assign(expr, locals, returned, next),
            K::Block { .. } | K::CallPreparation { .. } => {
                self.transfer_block(expr, locals, returned, next)
            }
            K::If { .. } => self.transfer_if(expr, locals, returned, next),
            K::Match { .. } => self.transfer_match(expr, locals, returned, next),
            K::Loop { .. } => self.transfer_loop(expr, locals, returned, next),
            K::Break { .. } | K::Continue { .. } => {
                self.transfer_break(expr, locals, returned, next)
            }
            K::Return { .. } => self.transfer_return(expr, locals, returned, next),
            K::Call { .. } => self.transfer_call(expr, locals, returned, next),
            K::TextQuery { .. } => self.transfer_text_query(expr, locals, returned, next),
            K::NewtypeConvert { .. } => self.transfer_newtype_convert(expr, locals, returned, next),
            K::ErrorConvert {
                conversion:
                    nir::error_results::Conversion::Embed | nir::error_results::Conversion::Product,
                ..
            } => self.transfer_error_embedding(expr, locals, returned, next),
            K::ErrorConvert { .. } => self.transfer_error_constructor(expr, locals, returned, next),
            K::Closure { .. } => self.transfer_closure(expr, locals, returned, next),
            K::Snapshot(value) | K::UnsafeScope(value) => self.expr(value, locals, returned, next),
            K::Discard(value)
            | K::RequirePredicate(value)
            | K::EnsurePredicate(value)
            | K::CheckPredicate(value)
            | K::Not(value)
            | K::FloatNegate(value)
            | K::IntegerNegate(value)
            | K::BitConvert(value)
            | K::TakeBits(value)
            | K::IntegerRepresentation(value)
            | K::TryIntegerConvert(value)
            | K::ComptimeIntegerConvert(value)
            | K::TypeKind { operand: value, .. }
            | K::TypeNotSized { operand: value }
            | K::TypeLayout { operand: value, .. }
            | K::BitAssign { value, .. }
            | K::BitSlice { value, .. } => Ok(self
                .expr(value, locals, returned, next)?
                .map(|_| Regions::new())),
            K::BitUpdate {
                selections, value, ..
            } => self.transfer_bit_update(None, selections, value, locals, returned, next),
            K::BitWritePointer {
                pointer,
                selections,
                value,
            } => {
                if let Some(message) = self.readonly_storage {
                    return Err(message.into());
                }
                self.transfer_bit_update(Some(pointer), selections, value, locals, returned, next)
            }
            K::IntegerBinary { .. }
            | K::FloatBinary { .. }
            | K::BitEqual { .. }
            | K::BitConcat { .. }
            | K::BitIndex { .. }
            | K::TypeEqual(..)
            | K::TemplateEqual { .. } => self.transfer_integer_binary(expr, locals, returned, next),
            K::BitWindow { .. } => self.transfer_bit_window(expr, locals, returned, next),
            K::Intrinsic { .. } => self.transfer_intrinsic(expr, locals, returned, next),
            K::And { .. } | K::Or { .. } => self.transfer_and(expr, locals, returned, next),
            K::ScalarEqual { .. }
            | K::ScalarLess { .. }
            | K::ScalarLessEqual { .. }
            | K::ScalarGreater { .. }
            | K::ScalarGreaterEqual { .. } => {
                self.transfer_scalar_equal(expr, locals, returned, next)
            }
            _ => Err(format!(
                "text region transfer is not implemented at {}",
                expr.source
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src() -> nir::program::SourceRange {
        nir::program::SourceRange { start: 0, end: 1 }
    }

    fn decl(local: u32) -> DeclarationId {
        DeclarationId { package: 0, local }
    }

    fn lid(index: u32) -> LocalId {
        LocalId { owner: 0, index }
    }

    fn expr(ty: Term, kind: K) -> Expr {
        Expr {
            source: src(),
            ty,
            kind,
        }
    }

    fn fn_decl(
        declaration: DeclarationId,
        name: &str,
        parameters: Vec<Term>,
        param_ids: Vec<LocalId>,
        result: Term,
    ) -> nir::program::FunctionDeclaration {
        nir::program::FunctionDeclaration {
            source: src(),
            phase: nir::program::FunctionPhase::Runtime,
            inline: false,
            pure: false,
            is_unsafe: false,
            preconditions: vec![],
            postconditions: vec![],
            result_binding: None,
            result_type_binder: None,
            result_guarantees: vec![],
            entry_snapshots: vec![],
            capabilities: vec![],
            requirements: vec![],
            type_parameters: vec![],
            declaration,
            name: name.into(),
            signature: Term::Function {
                parameters: parameters
                    .into_iter()
                    .map(|ty| nir::Parameter {
                        sort: nir::Sort::Runtime,
                        ty,
                    })
                    .collect(),
                result: Box::new(result),
            },
            parameters: param_ids,
            env_parameters: vec![],
            defaults: vec![],
        }
    }

    #[test]
    fn infer_text_regions_with_metadata_handles_comptime_type_equal_and_bit_ops() {
        let str_ty = Term::Primitive(Primitive::Str);
        let type_ty = Term::Primitive(Primitive::Type);
        let u8_ty = Term::Primitive(Primitive::U8);
        let bv4_ty = Term::Primitive(Primitive::BitVec(4));
        let bool_ty = Term::Primitive(Primitive::Bool);
        let fn_ty = Term::Function {
            parameters: vec![],
            result: Box::new(Term::Primitive(Primitive::Void)),
        };

        let comptime_str_fn = decl(1);
        let comptime_fn_fn = decl(2);
        let bit_and_type_eq_fn = decl(3);

        let local_bv = lid(1);
        let bv_place = nir::program::Place {
            root: local_bv,
            fields: vec![],
        };

        let package = Package {
            functions: vec![
                Function {
                    interface: fn_decl(comptime_str_fn, "ct_str", vec![], vec![], str_ty.clone()),
                    body: expr(str_ty.clone(), K::ComptimeValue(str_ty.clone())),
                    runtime_deliveries: Default::default(),
                },
                Function {
                    interface: fn_decl(comptime_fn_fn, "ct_fn", vec![], vec![], fn_ty.clone()),
                    body: expr(fn_ty.clone(), K::ComptimeValue(fn_ty.clone())),
                    runtime_deliveries: Default::default(),
                },
                Function {
                    interface: fn_decl(
                        bit_and_type_eq_fn,
                        "bit_and_type_eq",
                        vec![],
                        vec![],
                        str_ty.clone(),
                    ),
                    body: expr(
                        str_ty.clone(),
                        K::Block {
                            statements: vec![
                                expr(
                                    bv4_ty.clone(),
                                    K::Let {
                                        binding: local_bv,
                                        value: Box::new(expr(
                                            bv4_ty.clone(),
                                            K::ComptimeValue(u8_ty.clone()),
                                        )),
                                    },
                                ),
                                expr(
                                    Term::Primitive(Primitive::Void),
                                    K::BitAssign {
                                        place: bv_place.clone(),
                                        storage_type: bv4_ty.clone(),
                                        start: 0,
                                        end: 2,
                                        value: Box::new(expr(
                                            Term::Primitive(Primitive::BitVec(2)),
                                            K::Unit,
                                        )),
                                    },
                                ),
                                expr(
                                    bv4_ty.clone(),
                                    K::BitUpdate {
                                        place: bv_place,
                                        storage_type: bv4_ty.clone(),
                                        selections: vec![nir::program::BitSelection::Index(
                                            Box::new(expr(
                                                Term::Primitive(Primitive::Usize),
                                                K::Unit,
                                            )),
                                        )],
                                        value: Box::new(expr(
                                            Term::Primitive(Primitive::BitVec(1)),
                                            K::Unit,
                                        )),
                                    },
                                ),
                                expr(
                                    bool_ty,
                                    K::Discard(Box::new(expr(
                                        Term::Primitive(Primitive::Bool),
                                        K::TypeEqual(
                                            Box::new(expr(
                                                type_ty.clone(),
                                                K::TypeValue(u8_ty.clone()),
                                            )),
                                            Box::new(expr(
                                                type_ty.clone(),
                                                K::ComptimeValue(u8_ty),
                                            )),
                                        ),
                                    ))),
                                ),
                            ],
                            result: Some(Box::new(expr(
                                str_ty.clone(),
                                K::Call {
                                    callee: Box::new(expr(
                                        Term::Function {
                                            parameters: vec![],
                                            result: Box::new(str_ty),
                                        },
                                        K::Function(comptime_str_fn),
                                    )),
                                    arguments: vec![],
                                    env_arguments: vec![],
                                    parameter_bindings: vec![],
                                },
                            ))),
                        },
                    ),
                    runtime_deliveries: Default::default(),
                },
            ],
            ..Default::default()
        };

        let metadata = crate::declarations::Metadata::new(Some(&package));
        let mut fuel = 20_000;
        let summary = infer_text_regions_with_metadata(
            &package,
            &metadata,
            bit_and_type_eq_fn,
            &[],
            &mut fuel,
        )
        .expect("comptime str + bit ops + type equal succeed");
        let returned = summary.returned.expect("normal return");
        assert_eq!(
            returned.get(&vec![]),
            Some(&BTreeSet::from([TextRegionOrigin::Static]))
        );

        let err =
            infer_text_regions_with_metadata(&package, &metadata, comptime_fn_fn, &[], &mut fuel)
                .expect_err("comptime callable must fail closed");
        assert!(err.contains("callable"), "{err}");
    }

    #[test]
    fn non_text_pointer_ops_skip_descriptor_roots_while_text_pointers_fail_closed() {
        let pair_decl = decl(10);
        let pair_ty = Term::Nominal {
            declaration: pair_decl,
            arguments: vec![],
        };
        let str_ty = Term::Primitive(Primitive::Str);
        let u32_ty = Term::Primitive(Primitive::U32);
        let u8_ty = Term::Primitive(Primitive::U8);

        let ptr_u8_fn = decl(1);
        let local_u8_fn = decl(2);
        let ptr_str_fn = decl(3);
        let pair_num_fn = decl(4);
        let pair_str_fn = decl(5);

        let p0 = lid(0);
        let local_u8 = lid(1);

        let package = Package {
            structs: vec![nir::program::Struct {
                parameter_types: vec![],
                capabilities: vec![],
                requirements: vec![],
                type_parameters: vec![],
                declaration: pair_decl,
                name: "Pair".into(),
                fields: vec![("s".into(), str_ty.clone()), ("n".into(), u32_ty.clone())],
                packed: nir::program::PackedMode::None,
            }],
            functions: vec![
                Function {
                    interface: fn_decl(
                        ptr_u8_fn,
                        "read_u8",
                        vec![Term::Pointer(Box::new(u8_ty.clone()))],
                        vec![p0],
                        u8_ty.clone(),
                    ),
                    body: expr(
                        u8_ty.clone(),
                        K::ReadPointer(Box::new(expr(
                            Term::Pointer(Box::new(u8_ty.clone())),
                            K::Local(p0),
                        ))),
                    ),
                    runtime_deliveries: Default::default(),
                },
                Function {
                    interface: fn_decl(local_u8_fn, "addr_u8", vec![], vec![], u8_ty.clone()),
                    body: expr(
                        u8_ty.clone(),
                        K::Block {
                            statements: vec![expr(
                                u8_ty.clone(),
                                K::Let {
                                    binding: local_u8,
                                    value: Box::new(expr(u8_ty.clone(), K::Unit)),
                                },
                            )],
                            result: Some(Box::new(expr(
                                u8_ty.clone(),
                                K::ReadPointer(Box::new(expr(
                                    Term::Pointer(Box::new(u8_ty.clone())),
                                    K::AddressOf(Box::new(expr(u8_ty.clone(), K::Local(local_u8)))),
                                ))),
                            ))),
                        },
                    ),
                    runtime_deliveries: Default::default(),
                },
                Function {
                    interface: fn_decl(
                        ptr_str_fn,
                        "read_str",
                        vec![Term::Pointer(Box::new(str_ty.clone()))],
                        vec![p0],
                        str_ty.clone(),
                    ),
                    body: expr(
                        str_ty.clone(),
                        K::ReadPointer(Box::new(expr(
                            Term::Pointer(Box::new(str_ty.clone())),
                            K::Local(p0),
                        ))),
                    ),
                    runtime_deliveries: Default::default(),
                },
                Function {
                    interface: fn_decl(
                        pair_num_fn,
                        "read_pair_num",
                        vec![Term::Pointer(Box::new(pair_ty.clone()))],
                        vec![p0],
                        u32_ty.clone(),
                    ),
                    body: expr(
                        u32_ty.clone(),
                        K::ReadPointer(Box::new(expr(
                            Term::Pointer(Box::new(u32_ty.clone())),
                            K::PointerField {
                                pointer: Box::new(expr(
                                    Term::Pointer(Box::new(pair_ty.clone())),
                                    K::Local(p0),
                                )),
                                declaration: pair_decl,
                                index: 1,
                            },
                        ))),
                    ),
                    runtime_deliveries: Default::default(),
                },
                Function {
                    interface: fn_decl(
                        pair_str_fn,
                        "read_pair_str",
                        vec![Term::Pointer(Box::new(pair_ty.clone()))],
                        vec![p0],
                        str_ty.clone(),
                    ),
                    body: expr(
                        str_ty.clone(),
                        K::ReadPointer(Box::new(expr(
                            Term::Pointer(Box::new(str_ty.clone())),
                            K::PointerField {
                                pointer: Box::new(expr(
                                    Term::Pointer(Box::new(pair_ty)),
                                    K::Local(p0),
                                )),
                                declaration: pair_decl,
                                index: 0,
                            },
                        ))),
                    ),
                    runtime_deliveries: Default::default(),
                },
            ],
            ..Default::default()
        };

        let metadata = crate::declarations::Metadata::new(Some(&package));
        let mut fuel = 20_000;
        infer_text_regions_with_metadata(&package, &metadata, ptr_u8_fn, &[], &mut fuel)
            .expect("*u8 read succeeds without descriptor root");
        infer_text_regions_with_metadata(&package, &metadata, local_u8_fn, &[], &mut fuel)
            .expect("local u8 address_of succeeds without descriptor root");
        infer_text_regions_with_metadata(&package, &metadata, pair_num_fn, &[], &mut fuel)
            .expect("reading non-text field from opaque *Pair succeeds");

        let err = infer_text_regions_with_metadata(&package, &metadata, ptr_str_fn, &[], &mut fuel)
            .expect_err("opaque *str read must fail closed");
        assert!(err.contains("pointer target is unknown"), "{err}");

        let err =
            infer_text_regions_with_metadata(&package, &metadata, pair_str_fn, &[], &mut fuel)
                .expect_err("reading str field from opaque *Pair must fail closed");
        assert!(err.contains("pointer target is unknown"), "{err}");
    }

    #[test]
    fn intrinsic_returning_str_fails_closed() {
        let str_ty = Term::Primitive(Primitive::Str);
        let usize_ty = Term::Primitive(Primitive::Usize);
        let family_decl = decl(99);
        let tag = nir::intrinsics::Tag {
            family: family_decl,
            variant: 0,
        };
        let family = nir::program::Enum {
            is_intrinsic: true,
            parameter_types: vec![],
            capabilities: vec![],
            requirements: vec![],
            type_parameters: vec![],
            declaration: family_decl,
            name: "MemOps".into(),
            variants: vec![nir::program::Variant {
                name: "RawRead".into(),
                shape: nir::program::VariantShape::Unit,
                fields: vec![],
            }],
            groups: vec![],
            encoding: None,
            symbol_encoding: None,
        };
        let checked = nir::intrinsics::Registry::for_family(
            &family,
            0,
            nir::intrinsics::Operation::MemRawRead,
        )
        .unwrap()
        .check(tag, std::slice::from_ref(&usize_ty), &str_ty)
        .unwrap();
        let raw_read_fn = decl(1);
        let package = Package {
            enums: vec![family],
            functions: vec![Function {
                interface: fn_decl(raw_read_fn, "forged_str", vec![], vec![], str_ty.clone()),
                body: expr(
                    str_ty,
                    K::Intrinsic {
                        tag,
                        binding: Some(checked),
                        inputs: vec![expr(usize_ty, K::Unit)],
                    },
                ),
                runtime_deliveries: Default::default(),
            }],
            ..Default::default()
        };
        let metadata = crate::declarations::Metadata::new(Some(&package));
        let mut fuel = 10_000;
        let err =
            infer_text_regions_with_metadata(&package, &metadata, raw_read_fn, &[], &mut fuel)
                .expect_err("intrinsic returning str must fail closed");
        assert!(err.contains("backing-region provenance"), "{err}");
    }

    #[test]
    fn effect_handle_and_request_preserve_text_mutations_and_fail_closed_for_text_payloads() {
        let str_ty = Term::Primitive(Primitive::Str);
        let u32_ty = Term::Primitive(Primitive::U32);
        let void_ty = Term::Primitive(Primitive::Void);
        let effect_decl = decl(80);
        let s_local = lid(1);
        let ok_fn = decl(1);
        let bad_payload_fn = decl(2);
        let bad_result_fn = decl(3);

        let u32_handler_sig = Term::Function {
            parameters: vec![nir::Parameter {
                sort: nir::Sort::Runtime,
                ty: u32_ty.clone(),
            }],
            result: Box::new(void_ty.clone()),
        };
        let str_handler_sig = Term::Function {
            parameters: vec![nir::Parameter {
                sort: nir::Sort::Runtime,
                ty: str_ty.clone(),
            }],
            result: Box::new(void_ty.clone()),
        };

        let op = nir::effects::Operation {
            declaration: effect_decl,
            arguments: vec![],
        };

        let package = Package {
            functions: vec![
                Function {
                    interface: fn_decl(ok_fn, "effect_ok", vec![], vec![], str_ty.clone()),
                    body: expr(
                        str_ty.clone(),
                        K::Block {
                            statements: vec![
                                expr(
                                    str_ty.clone(),
                                    K::Let {
                                        binding: s_local,
                                        value: Box::new(expr(
                                            str_ty.clone(),
                                            K::Literal(nir::program::Literal::String("init".into())),
                                        )),
                                    },
                                ),
                                expr(
                                    void_ty.clone(),
                                    K::EffectHandle {
                                        computation: Box::new(expr(
                                            void_ty.clone(),
                                            K::EffectComputation {
                                                effects: vec![op.clone()],
                                                body: Box::new(expr(
                                                    void_ty.clone(),
                                                    K::EffectRequest {
                                                        operation: op.clone(),
                                                        abi: nir::effects::Abi::Direct,
                                                        allowed_index: 0,
                                                        signature: Box::new(
                                                            u32_handler_sig.clone(),
                                                        ),
                                                        arguments: vec![expr(u32_ty.clone(), K::Unit)],
                                                    },
                                                )),
                                            },
                                        )),
                                        handlers: vec![nir::program::EffectHandler {
                                            operation: op.clone(),
                                            abi: nir::effects::Abi::Direct,
                                            signature: u32_handler_sig,
                                            arms: vec![nir::program::MatchArm {
                                                pattern: nir::program::Pattern::Tuple(vec![
                                                    nir::program::Pattern::Wild,
                                                ]),
                                                guard: None,
                                                body: expr(
                                                    Term::Primitive(Primitive::Never),
                                                    K::Block {
                                                        statements: vec![expr(
                                                            void_ty.clone(),
                                                            K::Assign {
                                                                place: nir::program::Place {
                                                                    root: s_local,
                                                                    fields: vec![],
                                                                },
                                                                value: Box::new(expr(
                                                                    str_ty.clone(),
                                                                    K::Literal(
                                                                        nir::program::Literal::String(
                                                                            "armed".into(),
                                                                        ),
                                                                    ),
                                                                )),
                                                            },
                                                        )],
                                                        result: Some(Box::new(expr(
                                                            Term::Primitive(Primitive::Never),
                                                            K::EffectResume(Box::new(expr(
                                                                void_ty.clone(),
                                                                K::Unit,
                                                            ))),
                                                        ))),
                                                    },
                                                ),
                                            }],
                                            fallback: None,
                                        }],
                                        outer: vec![],
                                    },
                                ),
                            ],
                            result: Some(Box::new(expr(str_ty.clone(), K::Local(s_local)))),
                        },
                    ),
                    runtime_deliveries: Default::default(),
                },
                Function {
                    interface: fn_decl(
                        bad_payload_fn,
                        "effect_bad_payload",
                        vec![],
                        vec![],
                        void_ty.clone(),
                    ),
                    body: expr(
                        void_ty.clone(),
                        K::EffectHandle {
                            computation: Box::new(expr(void_ty.clone(), K::Unit)),
                            handlers: vec![nir::program::EffectHandler {
                                operation: op.clone(),
                                abi: nir::effects::Abi::Direct,
                                signature: str_handler_sig,
                                arms: vec![],
                                fallback: None,
                            }],
                            outer: vec![],
                        },
                    ),
                    runtime_deliveries: Default::default(),
                },
                Function {
                    interface: fn_decl(
                        bad_result_fn,
                        "effect_bad_result",
                        vec![],
                        vec![],
                        str_ty.clone(),
                    ),
                    body: expr(
                        str_ty.clone(),
                        K::EffectRequest {
                            operation: op,
                            abi: nir::effects::Abi::Direct,
                            allowed_index: 0,
                            signature: Box::new(Term::Function {
                                parameters: vec![],
                                result: Box::new(str_ty.clone()),
                            }),
                            arguments: vec![],
                        },
                    ),
                    runtime_deliveries: Default::default(),
                },
            ],
            ..Default::default()
        };

        let metadata = crate::declarations::Metadata::new(Some(&package));
        let mut fuel = 50_000;
        let summary = infer_text_regions_with_metadata(&package, &metadata, ok_fn, &[], &mut fuel)
            .expect("text-free effect handle with outer static str write succeeds");
        assert_eq!(
            summary.returned.expect("normal return").get(&vec![]),
            Some(&BTreeSet::from([TextRegionOrigin::Static]))
        );

        let err =
            infer_text_regions_with_metadata(&package, &metadata, bad_payload_fn, &[], &mut fuel)
                .expect_err("effect handler with str payload must fail closed");
        assert!(err.contains("text or callables"), "{err}");

        let err =
            infer_text_regions_with_metadata(&package, &metadata, bad_result_fn, &[], &mut fuel)
                .expect_err("effect request returning str must fail closed");
        assert!(err.contains("text or callables"), "{err}");
    }

    #[test]
    fn effect_computation_return_unwraps_for_text_layout_checks() {
        let str_ty = Term::Primitive(Primitive::Str);
        let void_ty = Term::Primitive(Primitive::Void);
        let effect_decl = decl(81);
        let op = nir::effects::Operation {
            declaration: effect_decl,
            arguments: vec![],
        };
        let void_comp = Term::Computation {
            effects: vec![op.clone()],
            result: Box::new(void_ty.clone()),
        };
        let str_comp = Term::Computation {
            effects: vec![op.clone()],
            result: Box::new(str_ty.clone()),
        };
        let ok_fn = decl(10);
        let bad_fn = decl(11);
        let package = Package {
            functions: vec![
                Function {
                    interface: fn_decl(ok_fn, "comp_void", vec![], vec![], void_comp.clone()),
                    body: expr(
                        void_comp,
                        K::EffectComputation {
                            effects: vec![op.clone()],
                            body: Box::new(expr(void_ty.clone(), K::Unit)),
                        },
                    ),
                    runtime_deliveries: Default::default(),
                },
                Function {
                    interface: fn_decl(
                        bad_fn,
                        "comp_str_no_origin",
                        vec![],
                        vec![],
                        str_comp.clone(),
                    ),
                    body: expr(
                        str_comp,
                        K::EffectComputation {
                            effects: vec![op],
                            body: Box::new(expr(str_ty.clone(), K::Tuple(vec![]))),
                        },
                    ),
                    runtime_deliveries: Default::default(),
                },
            ],
            ..Default::default()
        };

        let metadata = crate::declarations::Metadata::new(Some(&package));
        let mut fuel = 20_000;
        infer_text_regions_with_metadata(&package, &metadata, ok_fn, &[], &mut fuel).expect(
            "effectful void computation should unwrap before checking returned text layout",
        );

        let err = infer_text_regions_with_metadata(&package, &metadata, bad_fn, &[], &mut fuel)
            .expect_err("effectful str computation without backing origin must still fail closed");
        assert!(err.contains("backing-region origin"), "{err}");
    }

    #[test]
    fn text_region_cache_reuses_converged_roots_and_never_caches_errors() {
        let str_ty = Term::Primitive(Primitive::Str);
        let helper_fn = decl(1);
        let root_a_fn = decl(2);
        let root_b_fn = decl(3);
        let bad_fn = decl(4);

        let static_str = expr(
            str_ty.clone(),
            K::Literal(nir::program::Literal::String("hello".into())),
        );
        let mut stmts = Vec::new();
        let b0 = lid(1);
        for i in 0..120 {
            let local = lid(i + 1);
            stmts.push(expr(
                str_ty.clone(),
                K::Let {
                    binding: local,
                    value: Box::new(static_str.clone()),
                },
            ));
        }
        let call_helper = expr(
            str_ty.clone(),
            K::Call {
                callee: Box::new(expr(
                    Term::Function {
                        parameters: vec![],
                        result: Box::new(str_ty.clone()),
                    },
                    K::Function(helper_fn),
                )),
                arguments: vec![],
                env_arguments: vec![],
                parameter_bindings: vec![],
            },
        );
        let package = Package {
            functions: vec![
                Function {
                    interface: fn_decl(helper_fn, "helper", vec![], vec![], str_ty.clone()),
                    body: expr(
                        str_ty.clone(),
                        K::Block {
                            statements: stmts,
                            result: Some(Box::new(expr(str_ty.clone(), K::Local(b0)))),
                        },
                    ),
                    runtime_deliveries: Default::default(),
                },
                Function {
                    interface: fn_decl(root_a_fn, "root_a", vec![], vec![], str_ty.clone()),
                    body: call_helper.clone(),
                    runtime_deliveries: Default::default(),
                },
                Function {
                    interface: fn_decl(root_b_fn, "root_b", vec![], vec![], str_ty.clone()),
                    body: call_helper,
                    runtime_deliveries: Default::default(),
                },
                Function {
                    interface: fn_decl(bad_fn, "bad", vec![], vec![], str_ty.clone()),
                    body: expr(str_ty, K::Tuple(vec![])),
                    runtime_deliveries: Default::default(),
                },
            ],
            ..Default::default()
        };

        let metadata = crate::declarations::Metadata::new(Some(&package));
        let mut methods = MethodCallees::default();
        let mut layouts = TypeLayoutCache::default();
        let mut text_cache = TextRegionCache::default();

        // Failing analysis must not populate TextRegionCache.
        let err = infer_text_regions_with_context(
            &package,
            None,
            &metadata,
            &mut methods,
            Some(&mut layouts),
            Some(&mut text_cache),
            bad_fn,
            &[],
            &mut 20_000,
        )
        .unwrap_err();
        assert!(err.contains("backing-region origin"), "{err}");
        assert!(text_cache.is_empty());

        // Insufficient fuel must fail and also leave TextRegionCache untouched.
        assert!(
            infer_text_regions_with_context(
                &package,
                None,
                &metadata,
                &mut methods,
                Some(&mut layouts),
                Some(&mut text_cache),
                root_a_fn,
                &[],
                &mut 50,
            )
            .is_err()
        );
        assert!(text_cache.is_empty());

        let mut cold_fuel = 50_000;
        let expected = infer_text_regions_with_context(
            &package,
            None,
            &metadata,
            &mut methods,
            Some(&mut layouts),
            Some(&mut text_cache),
            root_a_fn,
            &[],
            &mut cold_fuel,
        )
        .expect("cold root_a succeeds");
        let cold_cost = 50_000 - cold_fuel;
        assert!(cold_cost > 200);
        assert_eq!(text_cache.len(), 2);

        // Warm call to root_b reuses converged helper summary under tiny budget (< cold_cost).
        let mut tight_callee = 30;
        let warm_b = infer_text_regions_with_context(
            &package,
            None,
            &metadata,
            &mut methods,
            Some(&mut layouts),
            Some(&mut text_cache),
            root_b_fn,
            &[],
            &mut tight_callee,
        )
        .expect("warm root_b reuses cached helper under tight budget");
        assert_eq!(warm_b.reads, expected.reads);

        // Warm root lookup also reuses cache directly and fails closed at 0 fuel.
        let mut tight_root = 5;
        let warm_a = infer_text_regions_with_context(
            &package,
            None,
            &metadata,
            &mut methods,
            Some(&mut layouts),
            Some(&mut text_cache),
            root_a_fn,
            &[],
            &mut tight_root,
        )
        .expect("warm root_a hits TextRegionCache directly");
        assert_eq!(warm_a, expected);
        assert!(
            infer_text_regions_with_context(
                &package,
                None,
                &metadata,
                &mut methods,
                Some(&mut layouts),
                Some(&mut text_cache),
                root_a_fn,
                &[],
                &mut 0,
            )
            .is_err()
        );
    }

    #[test]
    fn text_region_storage_index_avoids_quadratic_budget_across_distinct_storage_calls() {
        let box_decl = decl(10);
        let box_ty = Term::Nominal {
            declaration: box_decl,
            arguments: vec![],
        };
        let ptr_box_ty = Term::Pointer(Box::new(box_ty.clone()));
        let str_ty = Term::Primitive(Primitive::Str);
        let void_ty = Term::Primitive(Primitive::Void);
        let fn_sig = Term::Function {
            parameters: vec![nir::Parameter {
                sort: nir::Sort::Runtime,
                ty: ptr_box_ty.clone(),
            }],
            result: Box::new(void_ty.clone()),
        };

        let caller_fn = decl(1);
        let param_p = lid(0);
        let mut functions = Vec::new();
        let mut statements = Vec::new();
        for i in 0..60u32 {
            let callee_id = decl(100 + i);
            let p_arg = lid(10);
            functions.push(Function {
                interface: fn_decl(
                    callee_id,
                    &format!("touch_{i}"),
                    vec![ptr_box_ty.clone()],
                    vec![p_arg],
                    void_ty.clone(),
                ),
                body: expr(void_ty.clone(), K::Unit),
                runtime_deliveries: Default::default(),
            });
            statements.push(expr(
                void_ty.clone(),
                K::Call {
                    callee: Box::new(expr(fn_sig.clone(), K::Function(callee_id))),
                    arguments: vec![nir::program::CallArgument {
                        passing: nir::program::ArgumentPassing::Value,
                        parameter: 0,
                        value: expr(
                            ptr_box_ty.clone(),
                            K::AddressOf(Box::new(expr(box_ty.clone(), K::Local(param_p)))),
                        ),
                        is_default: false,
                    }],
                    env_arguments: vec![],
                    parameter_bindings: vec![p_arg],
                },
            ));
        }
        functions.push(Function {
            interface: fn_decl(
                caller_fn,
                "caller",
                vec![box_ty.clone()],
                vec![param_p],
                void_ty.clone(),
            ),
            body: expr(
                void_ty.clone(),
                K::Block {
                    statements,
                    result: Some(Box::new(expr(void_ty, K::Unit))),
                },
            ),
            runtime_deliveries: Default::default(),
        });

        let package = Package {
            structs: vec![nir::program::Struct {
                declaration: box_decl,
                name: "TextBox".into(),
                type_parameters: vec![],
                parameter_types: vec![],
                requirements: vec![],
                capabilities: vec![],
                fields: vec![("s".into(), str_ty)],
                packed: nir::program::PackedMode::None,
            }],
            functions,
            ..Default::default()
        };

        let metadata = crate::declarations::Metadata::new(Some(&package));
        // Quadratic storage scanning over 60 calls (each cost >= 15) exceeds 25_000;
        // hashed lookup uses O(N) fuel.
        let mut fuel = 12_000;
        infer_text_regions_with_metadata(&package, &metadata, caller_fn, &[], &mut fuel)
            .expect("60 distinct storage calls must succeed in linear budget");
        assert!(12_000 - fuel < 10_000, "used {}", 12_000 - fuel);
    }

    #[test]
    fn intrinsic_str_grounding_accepts_validated_inputs_and_storage_and_rejects_ungrounded() {
        let str_ty = Term::Primitive(Primitive::Str);
        let usize_ty = Term::Primitive(Primitive::Usize);
        let ptr_str_ty = Term::Pointer(Box::new(str_ty.clone()));
        let package = Package::default();
        let metadata = crate::declarations::Metadata::new(Some(&package));
        let mut methods = MethodCallees::default();
        let mut fuel = 20_000;

        let s_param = lid(1);
        let dead_local = lid(2);
        let mut locals = Locals::new();
        locals.insert(
            s_param,
            Regions::from(BTreeMap::from([(
                vec![],
                BTreeSet::from([TextRegionOrigin::Parameter {
                    parameter: 0,
                    fields: vec![],
                }]),
            )])),
        );

        let mut query = Query {
            package: &package,
            source: None,
            declarations: &metadata,
            cached_domains: None,
            fuel: &mut fuel,
            current: None,
            local_callables: BTreeMap::new(),
            closed_closures: vec![],
            storage_entries: vec![],
            storage_index: BTreeMap::new(),
            storage_pending: VecDeque::new(),
            storage_scheduled: BTreeSet::new(),
            storage_dependents: BTreeMap::new(),
            storage_cache_dependents: BTreeMap::new(),
            storage_outer_dependents: BTreeMap::new(),
            storage_outer_dependents_set: BTreeMap::new(),
            storage_current: None,
            solving_storage: false,
            next_storage_frame: 0,
            readonly_storage: None,
            changed_callables: BTreeSet::new(),
            pending: VecDeque::new(),
            scheduled: BTreeSet::new(),
            dependents: BTreeMap::new(),
            dependents_set: BTreeMap::new(),
            reads: vec![BTreeSet::new()],
            loops: vec![],
            exits: vec![],
            cache: BTreeMap::new(),
            methods: &mut methods,
            layout_cache: TypeLayoutCacheStorage::default(),
            external_cache: TextRegionCacheStorage::default(),
        };

        // Grounded directly from a validated `str` input region.
        let grounded_from_input = query
            .ground_intrinsic_text_outputs(
                &[expr(str_ty.clone(), K::Local(s_param))],
                &str_ty,
                &mut locals,
                &mut None,
                0,
            )
            .expect("grounded text input must propagate backing region")
            .expect("normal completion");
        assert_eq!(
            grounded_from_input.text.get(&vec![]),
            Some(&BTreeSet::from([TextRegionOrigin::Parameter {
                parameter: 0,
                fields: vec![],
            }]))
        );

        // Grounded from a live local storage descriptor pointing at `s_param`.
        let grounded_from_storage = query
            .ground_intrinsic_text_outputs(
                &[expr(
                    ptr_str_ty.clone(),
                    K::AddressOf(Box::new(expr(str_ty.clone(), K::Local(s_param)))),
                )],
                &str_ty,
                &mut locals,
                &mut None,
                0,
            )
            .expect("grounded storage descriptor must propagate backing region")
            .expect("normal completion");
        assert_eq!(
            grounded_from_storage.text.get(&vec![]),
            Some(&BTreeSet::from([TextRegionOrigin::Parameter {
                parameter: 0,
                fields: vec![],
            }]))
        );
        assert!(query.reads[0].contains(&TextRegionOrigin::Parameter {
            parameter: 0,
            fields: vec![],
        }));

        // Ungrounded `usize` input cannot synthesize `str`.
        let err = query
            .ground_intrinsic_text_outputs(
                &[expr(usize_ty, K::Unit)],
                &str_ty,
                &mut locals,
                &mut None,
                0,
            )
            .expect_err("ungrounded intrinsic output must fail closed");
        assert!(err.contains("backing-region provenance"), "{err}");

        // Live storage whose `str` has empty/cleared origins cannot synthesize `str`.
        locals.insert(dead_local, Regions::new());
        let err = query
            .ground_intrinsic_text_outputs(
                &[expr(
                    ptr_str_ty,
                    K::AddressOf(Box::new(expr(str_ty.clone(), K::Local(dead_local)))),
                )],
                &str_ty,
                &mut locals,
                &mut None,
                0,
            )
            .expect_err("empty text storage descriptor must fail closed");
        assert!(err.contains("backing-region provenance"), "{err}");
    }
}
