//! Reusable invocation of one concrete pure comptime function in an owned
//! compilation snapshot. Pure results whose representation is exportable are
//! memoized under versioned `(definitions, protocols, target)` scope fingerprints;
//! callers supply prepared arguments, while defaults and input evaluation remain
//! outside this cache. Runtime resource protocols do not authorize comptime
//! value-reference operations or cache admission.
use crate::comptime_keys::{Protocol, Snapshot};
use comptime_executor::{Cache, Context, Error, KeyProtocol, Outcome, QueryId, Scope};
use comptime_interpretor::{AdmittedCall, Budget, Interpreter, Value};
use elaboration::{
    materialization::{ClosedDeclarations, DeclarationSnapshot, DeclarationSource},
    ptc,
};
use intrinsic::LangItems;
use nir::{
    DeclarationId, Term,
    program::{FunctionPhase, Package, TypePredicate},
};

pub struct Invocation {
    declarations: ClosedDeclarations,
    code_session: comptime_interpretor::CodeSession,
    stats: crate::invocation::InvocationStats,
    declaration: DeclarationId,
    arguments: Vec<Term>,
    signature: Term,
    results: Results,
    budget: ptc::Budget,
}

struct Results {
    items: LangItems,
    schemas: Vec<Schema>,
    capacity: usize,
    protocols: ExactInputs,
    cache: Cache<Snapshot, Value>,
}

struct Schema {
    inputs: Vec<Term>,
    result: Term,
    protocols: Vec<usize>,
}

impl std::fmt::Display for Schema {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(binding (inputs")?;
        for ty in &self.inputs {
            write!(f, " {ty}")?;
        }
        write!(f, ") (result {}) (protocols", self.result)?;
        for slot in &self.protocols {
            write!(f, " {slot}")?;
        }
        f.write_str("))")
    }
}

struct ExactInputs(Vec<Protocol>);

impl ExactInputs {
    fn protocol(
        &mut self,
        value: &Snapshot,
        context: &mut Context,
    ) -> Result<&mut Protocol, Error<String>> {
        for protocol in &mut self.0 {
            context.spend::<String>(1)?;
            if protocol.owns(value) {
                return Ok(protocol);
            }
        }
        Err(Error::Protocol(
            "input snapshot has no protocol in this invocation".into(),
        ))
    }
}

impl KeyProtocol<Snapshot> for ExactInputs {
    type Error = String;

    fn hash(&mut self, value: &Snapshot, context: &mut Context) -> Result<u64, Error<String>> {
        self.protocol(value, context)?.hash(value, context)
    }

    fn equivalent(
        &mut self,
        a: &Snapshot,
        b: &Snapshot,
        context: &mut Context,
    ) -> Result<bool, Error<String>> {
        if !self.protocol(a, context)?.equivalent(a, b, context)? {
            return Ok(false);
        }
        // Source Eq remains required. This additional dependency discriminator
        // establishes compatibility for queries that can inspect every field.
        // It is intentionally not an alternative source equality definition.
        context.spend::<String>(a.expanded_size() + b.expanded_size())?;
        Ok(a.value() == b.value())
    }
}

impl Invocation {
    /// Capture and prepare one invocation under a shared construction allowance.
    /// Later execution uses its own cumulative Context and retained query limits.
    pub fn prepare(
        package: &Package,
        items: &LangItems,
        declaration: DeclarationId,
        arguments: Vec<Term>,
        capacity: usize,
        budget: ptc::Budget,
    ) -> Result<Self, String> {
        let mut remaining = budget.steps;
        let source = DeclarationSource::Borrowed(package)
            .retain_declarations(&mut remaining)
            .map_err(|error| format!("memoization source retention budget exhausted: {error:?}"))?;
        Self::prepare_in_source_metered(
            source,
            items,
            declaration,
            arguments,
            capacity,
            budget,
            &mut remaining,
        )
    }

    /// Reuse the exact source owner across callers. Preparation proves this
    /// invocation's requirements; sharing a source grants no cache admission.
    pub fn prepare_in_source(
        source: DeclarationSnapshot,
        items: &LangItems,
        declaration: DeclarationId,
        arguments: Vec<Term>,
        capacity: usize,
        budget: ptc::Budget,
    ) -> Result<Self, String> {
        Self::prepare_in_source_metered(
            source,
            items,
            declaration,
            arguments,
            capacity,
            budget,
            &mut { budget.steps },
        )
    }

    /// Report construction work without reducing the independent query limits
    /// retained for future invocations. Failed preparation publishes no cache.
    pub fn prepare_in_source_metered(
        source: DeclarationSnapshot,
        items: &LangItems,
        declaration: DeclarationId,
        arguments: Vec<Term>,
        capacity: usize,
        budget: ptc::Budget,
        remaining: &mut usize,
    ) -> Result<Self, String> {
        let mut work = ptc::Budget {
            steps: budget.steps.min(*remaining),
            ..budget
        };
        let before = work.steps;
        let prepared = (|| {
            let declarations = source
                .closed(&mut work)
                .map_err(|error| format!("memoization declaration preparation: {error}"))?;
            let function = declarations
                .function(declaration, &mut work.steps)?
                .ok_or("memoized function is missing")?;
            if function.phase != FunctionPhase::Comptime || !function.pure {
                return Err("memoization requires one ordinary pure comptime function".into());
            }
            let frames =
                [
                    nir::function_parameters::ordinary_frame(&function.signature, &mut work.steps)
                        .map_err(nir::NominalAdmissionError::from)?,
                ];
            let inputs = declarations.world().admit_inputs(
                &function.type_parameters,
                &arguments,
                nir::DeclarationInputScope::ClosedInBody(&frames),
                &mut work.steps,
            )?;
            let signature = inputs
                .instantiate_in_input_scope(&function.signature, &mut work.steps)
                .map_err(nir::NominalAdmissionError::from)?;
            let Term::Function { parameters, result } = &signature else {
                return Err("memoized function signature is invalid".into());
            };
            signature.check_scope(&[]).map_err(|e| e.to_string())?;
            let closed = parameters
                .iter()
                .map(|p| &p.ty)
                .chain(std::iter::once(result.as_ref()))
                .all(|ty| ty.check_scope(&[]).is_ok());
            let world = declarations.capabilities();
            work.steps = work
                .steps
                .checked_sub(
                    function
                        .requirements
                        .len()
                        .saturating_add(function.capabilities.len()),
                )
                .ok_or("memoized requirement collection budget exhausted")?;
            let check_requirement = |predicate: TypePredicate,
                                     work: &mut ptc::Budget|
             -> Result<(), String> {
                let captures =
                    nir::copy_budget::predicate_capture_slots(&predicate, &frames, &mut work.steps)
                        .map_err(nir::NominalAdmissionError::from)?;
                if captures.is_empty()
                    && !matches!(
                        world.check_predicate_metered(&predicate, work)?,
                        ptc::Outcome::Entailed(_)
                    )
                {
                    return Err("memoized function type contract is not established".into());
                }
                Ok(())
            };
            for requirement in &function.requirements {
                let bound = inputs
                    .instantiate_predicate(requirement, &mut work.steps)
                    .map_err(nir::NominalAdmissionError::from)?;
                check_requirement(bound, &mut work)?;
            }
            for capability in &function.capabilities {
                let bound = inputs
                    .instantiate_requirement(capability, &mut work.steps)
                    .map_err(nir::NominalAdmissionError::from)?;
                check_requirement(TypePredicate::Trait(bound), &mut work)?;
            }
            let cacheable = closed;
            let scope = scope_for(source.package(), items);
            let code_session = comptime_interpretor::CodeSession::new(source.package().clone());
            let protocols = if cacheable {
                parameters
                    .iter()
                    .map(|p| {
                        Protocol::prepare_in_source_with_session_metered(
                            source.clone(),
                            items,
                            &p.ty,
                            budget,
                            &mut work.steps,
                            &code_session,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?
            } else {
                vec![]
            };
            let schemas = if cacheable {
                work.steps = work
                    .steps
                    .checked_sub(parameters.len().saturating_add(protocols.len()))
                    .ok_or("memoized schema budget exhausted")?;
                for ty in parameters
                    .iter()
                    .map(|p| &p.ty)
                    .chain(std::iter::once(result.as_ref()))
                {
                    nir::copy_budget::charge_type_substitution(ty, &[], &[], &mut work.steps)
                        .map_err(|error| {
                            format!("memoized schema copy budget exhausted: {error:?}")
                        })?;
                }
                vec![Schema {
                    inputs: parameters.iter().map(|p| p.ty.clone()).collect(),
                    result: (**result).clone(),
                    protocols: (0..protocols.len()).collect(),
                }]
            } else {
                vec![]
            };
            work.steps = work
                .steps
                .checked_sub(items.len())
                .ok_or("memoized language bindings budget exhausted")?;
            Ok(Self {
                declarations,
                code_session,
                stats: Default::default(),
                declaration,
                arguments,
                signature,
                results: Results {
                    items: items.clone(),
                    schemas,
                    capacity,
                    protocols: ExactInputs(protocols),
                    cache: Cache::new(scope, capacity),
                },
                budget,
            })
        })();
        *remaining -= before - work.steps;
        prepared
    }

    pub fn last_execution_stats(&self) -> crate::invocation::InvocationStats {
        self.stats
    }

    pub fn cached_results(&self) -> usize {
        self.results.cache.len()
    }

    pub fn scope(&self) -> Scope {
        self.results.cache.scope()
    }

    pub fn set_scope(&mut self, scope: Scope) -> bool {
        let changed = self.results.cache.scope() != scope;
        if changed {
            self.results.cache.set_scope(scope);
            self.results.schemas.clear();
            self.results.protocols.0.clear();
        }
        changed
    }

    pub fn invoke(
        &mut self,
        values: Vec<Value>,
        context: &mut Context,
    ) -> Result<Outcome<Value>, Error<String>> {
        self.stats = Default::default();
        let package = self.declarations.source().package();
        let queries =
            elaboration::materialization::SourceInvocationQueries::in_closed(&self.declarations);
        let budget = self.budget;
        let solver =
            |predicate: &TypePredicate, steps: &mut u64| queries.check(predicate, budget, steps);
        let methods = |callee: &nir::program::Expr, steps: &mut u64| queries.resolve(callee, steps);
        let associated =
            |access: &nir::program::Expr, steps: &mut u64| queries.associated(access, steps);
        let mut allowance = Budget {
            steps: context.remaining() as u64,
            depth: 128,
        };
        let before = allowance.steps;
        let vm = Interpreter::for_comptime_resolving_in_session(
            package,
            &mut self.code_session,
            comptime_interpretor::SourceResolvers {
                types: &solver,
                methods: &methods,
                associated: &associated,
            },
            &mut allowance,
        );
        context.spend::<String>((before - allowance.steps) as usize)?;
        let mut vm = vm.map_err(evaluation_error)?;
        let result = vm.with_code_session_source(&mut self.code_session, |vm, source_session| {
            let entry = context.with_allowance(usize::MAX, |remaining| {
                let arguments = self
                    .arguments
                    .iter()
                    .map(|arg| nir::copy_budget::clone_term(arg, remaining))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|_| Error::BudgetExhausted)?;
                vm.admit_source_call(self.declaration, arguments, values, remaining)
                    .map_err(admission_error)
            })?;
            self.results.lookup(
                entry,
                self.declaration,
                self.declarations.source(),
                source_session,
                budget,
                context,
            )
        });
        self.stats = crate::invocation::InvocationStats {
            bytecode_compilations: vm.bytecode_compilations(),
            bytecode_invocations: vm.bytecode_invocations(),
        };
        result
    }
}

impl Results {
    fn lookup(
        &mut self,
        entry: AdmittedCall<'_, '_>,
        declaration: DeclarationId,
        source: &DeclarationSnapshot,
        session: &comptime_interpretor::CodeSession,
        budget: ptc::Budget,
        context: &mut Context,
    ) -> Result<Outcome<Value>, Error<String>> {
        let types = entry.domains();
        let mut transient = None;
        let (operation, inputs) = if let Some(index) = self
            .schemas
            .iter()
            .position(|schema| schema.inputs == types && &schema.result == entry.result_type())
        {
            let inputs = entry
                .values()
                .zip(&self.schemas[index].protocols)
                .map(|(value, &slot)| self.protocols.0[slot].admit_borrowed(value, context))
                .collect::<Result<Vec<_>, _>>()?;
            (index as u32, inputs)
        } else {
            context.spend::<String>(types.len().saturating_add(1))?;
            let mut protocols = context
                .with_allowance(budget.steps, |work| {
                    types
                        .iter()
                        .map(|ty| {
                            Protocol::prepare_in_source_with_session_metered(
                                source.clone(),
                                &self.items,
                                ty,
                                budget,
                                work,
                                session,
                            )
                        })
                        .collect::<Result<Vec<_>, _>>()
                })
                .map_err(Error::Protocol)?;
            let inputs = entry
                .values()
                .zip(&mut protocols)
                .map(|(value, protocol)| protocol.admit_borrowed(value, context))
                .collect::<Result<Vec<_>, _>>()?;
            if self.schemas.len() < self.capacity && self.schemas.len() < u32::MAX as usize {
                let (types, result) = context
                    .with_allowance(usize::MAX, |work| {
                        let types = types
                            .iter()
                            .map(|ty| nir::copy_budget::clone_term(ty, work))
                            .collect::<Result<Vec<_>, _>>()?;
                        let result = nir::copy_budget::clone_term(entry.result_type(), work)?;
                        Ok::<_, nir::copy_budget::CopyBudgetExhausted>((types, result))
                    })
                    .map_err(|_| Error::BudgetExhausted)?;
                let operation = self.schemas.len() as u32;
                let start = self.protocols.0.len();
                let slots = (start..start + protocols.len()).collect();
                self.protocols.0.extend(protocols);
                self.schemas.push(Schema {
                    inputs: types,
                    result,
                    protocols: slots,
                });
                (operation, inputs)
            } else {
                transient = Some(ExactInputs(protocols));
                (0, inputs)
            }
        };
        let key = QueryId {
            package: declaration.package,
            declaration: declaration.local,
            operation,
        };
        let mut uncached;
        let (cache, protocols) = if let Some(ref mut protocols) = transient {
            uncached = Cache::new(self.cache.scope(), 0);
            (&mut uncached, protocols)
        } else {
            (&mut self.cache, &mut self.protocols)
        };
        cache.query_pure_admitted(key, inputs, context, protocols, |_, context| {
            context
                .with_allowance(usize::MAX, |remaining| entry.resume_for_export(remaining))
                .map_err(evaluation_error)
        })
    }
}

impl std::fmt::Display for Invocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(memoized-invocation (declaration {}) (arguments",
            self.declaration
        )?;
        for argument in &self.arguments {
            write!(f, " {argument}")?;
        }
        write!(
            f,
            ") (signature {}) (cached-results {}) (bindings",
            self.signature,
            self.results.cache.len()
        )?;
        for schema in &self.results.schemas {
            write!(f, " {schema}")?;
        }
        f.write_str("))")
    }
}

fn admission_error(error: comptime_interpretor::Error) -> Error<String> {
    use comptime_interpretor::ErrorKind;
    match error.kind {
        ErrorKind::TypeContractRejected | ErrorKind::TypeContractUnresolved => Error::Protocol(
            format!("memoized function bound type contract is not established: {error}"),
        ),
        ErrorKind::Type | ErrorKind::Arity | ErrorKind::FunctionInputRejected(_) => {
            Error::Protocol(error.to_string())
        }
        _ => evaluation_error(error),
    }
}

fn evaluation_error(error: comptime_interpretor::Error) -> Error<String> {
    use comptime_interpretor::ErrorKind;
    if matches!(
        error.kind,
        ErrorKind::StepsExhausted | ErrorKind::DepthExhausted
    ) {
        return Error::BudgetExhausted;
    }
    Error::Evaluation(error.to_string())
}

struct HashWriter(u64);

impl HashWriter {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    fn byte(&mut self, byte: u8) {
        self.0 ^= u64::from(byte);
        self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
    }

    fn u64(&mut self, value: u64) {
        for byte in value.to_le_bytes() {
            self.byte(byte);
        }
    }

    fn display(&mut self, value: &impl std::fmt::Display) {
        let _ = std::fmt::Write::write_fmt(self, format_args!("{value}"));
        self.byte(0xff);
    }
}

impl std::fmt::Write for HashWriter {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        for &byte in s.as_bytes() {
            self.byte(byte);
        }
        Ok(())
    }
}

/// Compute versioned `(definitions, protocols, target)` scope fingerprints for a
/// compilation snapshot and language-item registry.
pub fn scope_for(package: &Package, items: &LangItems) -> Scope {
    use intrinsic::{LangItem, LangItemDef};

    let mut defs = HashWriter::new();
    defs.u64(u64::from(package.definition_identity_cursor));
    defs.display(&package.observed_associated_types);
    defs.display(&package.formed_nullary_nominals);
    defs.display(&package.refinements);
    for effect in &package.effects {
        defs.display(effect);
    }
    for identity in &package.runtime_function_identities {
        defs.display(identity);
    }
    for decl in &package.function_parameters {
        defs.display(decl);
    }
    for head in &package.normal_forms {
        defs.display(head);
    }
    for enum_def in &package.enums {
        defs.display(enum_def);
    }
    for contract in &package.alias_contracts {
        defs.display(contract);
    }
    for newtype in &package.newtypes {
        defs.display(newtype);
    }
    for struct_def in &package.structs {
        defs.display(struct_def);
    }
    for function in &package.functions {
        defs.display(function);
    }
    for static_def in &package.statics {
        let _ = std::fmt::Write::write_fmt(&mut defs, format_args!("{static_def:?}"));
        defs.byte(0xff);
    }
    defs.display(&package.definition_attributes);
    defs.display(&package.volatile_storage);
    for (pkg_id, meta) in &package.package_metadata {
        defs.u64(*pkg_id);
        defs.display(meta);
    }
    for (decl_id, path) in &package.declaration_paths {
        defs.display(decl_id);
        defs.display(path);
    }

    let mut protos = HashWriter::new();
    const ALL_ITEMS: [LangItem; 29] = [
        LangItem::AnyType,
        LangItem::U8,
        LangItem::U16,
        LangItem::U32,
        LangItem::U64,
        LangItem::U128,
        LangItem::Usize,
        LangItem::I8,
        LangItem::I16,
        LangItem::I32,
        LangItem::I64,
        LangItem::I128,
        LangItem::Isize,
        LangItem::Integer,
        LangItem::F16,
        LangItem::Bf16,
        LangItem::F32,
        LangItem::F64,
        LangItem::Real,
        LangItem::Bool,
        LangItem::Char,
        LangItem::Str,
        LangItem::Void,
        LangItem::NoReturn,
        LangItem::Type,
        LangItem::Tuple,
        LangItem::Undefined,
        LangItem::AnyValue,
        LangItem::Once,
    ];
    const TRAIT_ITEMS: [LangItem; 13] = [
        LangItem::Copy,
        LangItem::Clone,
        LangItem::Drop,
        LangItem::Debug,
        LangItem::Display,
        LangItem::Eq,
        LangItem::Ord,
        LangItem::Hash,
        LangItem::Iter,
        LangItem::IntoIterator,
        LangItem::Into,
        LangItem::From,
        LangItem::Splice,
    ];
    for (index, item) in ALL_ITEMS.into_iter().chain(TRAIT_ITEMS).enumerate() {
        protos.u64(index as u64);
        match items.get(item) {
            None => protos.byte(0),
            Some(LangItemDef::Builtin) => protos.byte(1),
            Some(LangItemDef::UserDef(id)) => {
                protos.byte(2);
                protos.u64(id.package);
                protos.u64(u64::from(id.local));
            }
        }
    }
    for pkg in &package.builtin_implementation_packages {
        protos.u64(*pkg);
    }
    for trait_def in &package.traits {
        protos.display(trait_def);
    }
    for implementation in &package.implementations {
        protos.display(implementation);
    }
    for link in &package.method_links {
        protos.display(link);
    }
    for instance in &package.dispatch_instances {
        protos.display(instance);
    }

    let mut target = HashWriter::new();
    match &package.target {
        None => target.byte(0),
        Some(data) => {
            target.byte(1);
            target.display(data);
        }
    }

    Scope {
        definitions: defs.0,
        protocols: protos.0,
        target: target.0,
    }
}

#[cfg(test)]
mod tests {
    use super::scope_for;
    use intrinsic::LangItems;
    use nir::{
        DeclarationId,
        program::{CanonicalDeclarationPath, Package, PackageMetadata},
    };

    #[test]
    fn scope_for_includes_package_metadata_and_declaration_paths() {
        let items = LangItems::default();
        let mut pkg = Package::default();
        let base = scope_for(&pkg, &items);

        pkg.package_metadata.insert(
            1,
            PackageMetadata {
                name: "acme".into(),
                version: "1.0.0".into(),
            },
        );
        let with_meta_v1 = scope_for(&pkg, &items);
        assert_ne!(base.definitions, with_meta_v1.definitions);

        pkg.package_metadata.get_mut(&1).unwrap().version = "1.2.0".into();
        let with_meta_v2 = scope_for(&pkg, &items);
        assert_ne!(with_meta_v1.definitions, with_meta_v2.definitions);

        pkg.declaration_paths.insert(
            DeclarationId {
                package: 1,
                local: 7,
            },
            CanonicalDeclarationPath {
                package: 1,
                modules: vec!["collections".into()],
                name: "Buffer".into(),
            },
        );
        let with_decl_path = scope_for(&pkg, &items);
        assert_ne!(with_meta_v2.definitions, with_decl_path.definitions);
    }
}
