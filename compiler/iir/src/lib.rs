//! Concrete high-level instances. Structured NIR operations are retained; this
//! stage neither invents SSA names nor erases program contracts or opaque heads.
mod index;
pub mod visit;
pub mod staging;
pub mod effect_values;
mod effects;
use nir::program::{DispatchInstance, Function};
use nir::{DeclarationId, RuntimeId, Term};
use std::{collections::BTreeSet, fmt};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct InstanceKey {
    pub declaration: DeclarationId,
    pub arguments: Vec<Term>,
}
impl fmt::Display for InstanceKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(instance-key (declaration {}) (arguments",
            self.declaration
        )?;
        for argument in &self.arguments {
            write!(f, " {argument}")?;
        }
        f.write_str("))")
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct FunctionInstance {
    pub key: InstanceKey,
    /// All declaration parameters have been substituted, including contracts,
    /// capture types and default expressions. Local IDs retain source identity.
    pub function: Function,
}
impl fmt::Display for FunctionInstance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(function-instance {} {})", self.key, self.function)
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum NominalDefinition {
    Struct(nir::program::Struct),
    Enum(nir::program::Enum),
}
impl fmt::Display for NominalDefinition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Struct(s) => s.fmt(f),
            Self::Enum(e) => e.fmt(f),
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct NominalInstance {
    pub ty: Term,
    pub definition: NominalDefinition,
}
impl fmt::Display for NominalInstance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(nominal-instance (type {}) {})",
            self.ty, self.definition
        )
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct InterfaceContract {
    pub interface: nir::program::InterfaceInstance,
    pub subject: Term,
    pub methods: Vec<nir::program::MethodContract>,
}
impl fmt::Display for InterfaceContract {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(interface-contract (interface {}) (subject {}) (methods",
            self.interface, self.subject
        )?;
        for method in &self.methods {
            write!(f, " {method}")?;
        }
        f.write_str("))")
    }
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Package {
    pub expansion_sources: Vec<nir::program::ExpansionSource>,
    pub roots: Vec<InstanceKey>,
    pub nominals: Vec<NominalInstance>,
    pub interfaces: Vec<InterfaceContract>,
    pub normal_forms: Vec<nir::normal_forms::Head>,
    pub functions: Vec<FunctionInstance>,
    /// Fully selected implementation evidence; no runtime implementation search.
    pub dispatch: Vec<DispatchInstance>,
    /// Explicitly supplied external runtime identities. Availability is not an
    /// ownership/lifetime proof; responsible resource passes retain that duty.
    pub runtime_bindings: BTreeSet<RuntimeId>,
}
impl Package {
    pub fn function(&self, key: &InstanceKey) -> Option<&FunctionInstance> {
        self.functions.iter().find(|function| &function.key == key)
    }
    /// Validate completed annotations and closure of every named callable
    /// reference, including references embedded in defaults and patterns.
    pub fn validate(&self, fuel: &mut usize) -> Result<(), String> {
        self.validate_index()?;
        for nominal in &self.nominals {
            self.validate_type(&nominal.ty, fuel)?;
            match &nominal.definition {
                NominalDefinition::Struct(s) => {
                    for (_, ty) in &s.fields {
                        self.validate_type(ty, fuel)?;
                    }
                }
                NominalDefinition::Enum(e) => {
                    for variant in &e.variants {
                        for ty in &variant.fields {
                            self.validate_type(ty, fuel)?;
                        }
                    }
                }
            }
        }
        for interface in &self.interfaces {
            for argument in &interface.interface.arguments {
                self.validate_type(argument, fuel)?;
            }
            self.validate_type(&interface.subject, fuel)?;
            for method in &interface.methods {
                self.validate_type(&method.signature, fuel)?;
                for argument in &method.type_arguments { self.validate_type(argument, fuel)?; }
                let summary = method
                    .program_summary(&[], &[])
                    .map_err(|e| e.to_string())?;
                visit::expressions(&summary, fuel, |expr, fuel| {
                    self.validate_expression_types(expr, fuel)
                })?;
            }
        }
        use nir::program::ExprKind as K;
        for instance in &self.functions {
            self.validate_type(&instance.function.signature, fuel)?;
            for argument in &instance.key.arguments {
                self.validate_type(argument, fuel)?;
            }
            visit::expressions(&instance.function, fuel, |expr, fuel| {
                self.validate_expression_types(expr, fuel)?;
                if let Some(message)=nir::program::pointer_operation_error(expr) {return Err(message.into());}
                let target = match &expr.kind {
                    K::Function(id) => Some(InstanceKey {
                        declaration: *id,
                        arguments: vec![],
                    }),
                    K::FunctionInstance {
                        declaration,
                        arguments,
                    } => Some(InstanceKey {
                        declaration: *declaration,
                        arguments: arguments.clone(),
                    }),
                    K::TraitMethod {
                        method_arguments,
                        interface,
                        interface_arguments,
                        member,
                        subject,
                    } => {
                        if self.interfaces.iter().any(|contract|
                            contract.interface.declaration==*interface
                            && contract.interface.arguments==*interface_arguments
                            && contract.subject==*subject
                            && contract.methods.iter().any(|method|method.declaration==*member && method.is_quote)) {
                            return Err("IIR quote method requires completed deterministic expansion".into());
                        }
                        let selected = self
                            .dispatch
                            .iter()
                            .find(|d| {
                                d.interface == *interface
                                    && d.interface_arguments == *interface_arguments
                                    && d.subject == *subject
                            })
                            .ok_or("IIR trait reference has no selected implementation")?;
                        let method = selected
                            .methods
                            .iter()
                            .find(|m| m.member == *member && m.method_arguments == *method_arguments)
                            .ok_or("IIR trait reference has no selected method")?;
                        Some(InstanceKey {
                            declaration: method.body,
                            arguments: method.arguments.clone(),
                        })
                    }

                    K::AssociatedValue {..} => return Err("IIR requires associated value selection".into()),
                    _ => None,
                };
                if let Some(target) = target {
                    let target = self
                        .function(&target)
                        .ok_or("IIR callable reference is dangling")?;
                    let comptime = target.function.phase == nir::program::FunctionPhase::Comptime;
                    if comptime != expr.ty.is_comptime_callable() {
                        return Err("IIR callable reference changed execution phase".into());
                    }
                }
                Ok(())
            })?;
            effects::validate(self, &instance.function, fuel)?;
        }
        effects::validate_contracts(self, fuel)?;
        Ok(())
    }
    /// Effect control exits need acquired cleanup schedules before this graph
    /// may participate in resource-checked execution or destructor invocation.
    pub fn validate_cleanup_control(&self, fuel: &mut usize) -> Result<(), String> {
        for instance in &self.functions {
            visit::expressions(&instance.function, fuel, |expr, fuel| {
                use nir::program::ExprKind as K;
                match &expr.kind {
                    K::EffectHandle { handlers, .. } => {
                        for handler in handlers {
                            if handler.abi != nir::effects::Abi::Direct
                                && !(handler.abi == nir::effects::Abi::DirectPassing
                                    && nir::effects::scalar_passing_signature(&handler.signature)) {
                                return Err("IIR effect installation requires slot/capture cleanup protocol lowering".into());
                            }
                            // Capture ownership/initialization is discharged by
                            // resource schedule publication, not type-only IIR
                            // validation. Traverse here to retain bounded lexical
                            // well-formedness checks before execution.
                            for arm in &handler.arms {
                                nir::free_bindings::collect_arm(arm, fuel)?;
                            }
                        }
                    }
                    K::EffectRequest { abi, signature, .. } if *abi != nir::effects::Abi::Direct
                        && !(*abi == nir::effects::Abi::DirectPassing
                            && nir::effects::scalar_passing_signature(signature)) => {
                        return Err("IIR effect request requires slot/capture cleanup protocol lowering".into());
                    }
                    _ => {}
                }
                Ok(())
            })?;
        }
        Ok(())
    }
    // Contracts and executable bodies must inspect the same embedded types.
    // Validating only Expr::ty misses constants and capture/storage annotations.
    fn validate_expression_types(&self, expr: &nir::program::Expr, fuel: &mut usize) -> Result<(), String> {
        use nir::program::ExprKind as K;
        self.validate_type(&expr.ty, fuel)?;
        match &expr.kind {
            K::TypeListValue(types) => {
                let mut admitted = false;
                for head in &self.normal_forms {
                    *fuel = fuel.checked_sub(1).ok_or("IIR type-list metadata budget exhausted")?;
                    for parameter in &head.parameters {
                        *fuel = fuel.checked_sub(1).ok_or("IIR type-list metadata budget exhausted")?;
                        admitted |= parameter.kind == nir::normal_forms::Kind::TypeList && parameter.ty == expr.ty;
                    }
                }
                if !admitted || !nir::normal_forms::type_list_input_type(&expr.ty) {
                    return Err("IIR type-list constant requires trusted meta.List<Type> metadata".into());
                }
                for ty in types {
                    self.validate_type(ty, fuel)?;
                    if ty.sort() != nir::Sort::Type { return Err("IIR type-list element is not Type".into()); }
                }
            }
            K::TypeValue(ty) | K::BitAssign {storage_type: ty, ..} | K::BitUpdate {storage_type: ty, ..} => self.validate_type(ty, fuel)?,
            K::Closure {captures, ..} => {
                for capture in captures { self.validate_type(&capture.ty, fuel)?; }
            }
            _ => {}
        }
        Ok(())
    }
    fn validate_type(&self, ty: &Term, fuel: &mut usize) -> Result<(), String> {
        completed_type(ty, &self.runtime_bindings, fuel)?;
        self.type_links(ty, fuel)
    }
    fn type_links(&self, ty: &Term, fuel: &mut usize) -> Result<(), String> {
        let mut pending = vec![ty];
        while let Some(ty) = pending.pop() {
            if *fuel == 0 {
                return Err("IIR type link budget exhausted".into());
            }
            *fuel -= 1;
            match ty {
                Term::Computation {effects,result} => {
                    pending.push(result);
                    for operation in effects {pending.extend(&operation.arguments);}
                }
                Term::Nominal { arguments, .. } => {
                    if !self.nominals.iter().any(|n| &n.ty == ty) {
                        return Err("IIR nominal reference is dangling".into());
                    }
                    pending.extend(arguments);
                }
                Term::NormalForm {
                    head, arguments, ..
                } => {
                    if !self.normal_forms.iter().any(|h| h.declaration == *head) {
                        return Err("IIR normal-form reference is dangling".into());
                    }
                    pending.extend(arguments);
                }
                Term::Tuple(fields) | Term::TypeListValue(fields) | Term::TypeListConcat(fields) | Term::TupleValue(fields) => pending.extend(fields),
                Term::NominalValue { ty, fields, .. } => { pending.push(ty); pending.extend(fields); },
                Term::Record(fields) | Term::RecordValue(fields) => pending.extend(fields.iter().map(|(_, ty)| ty)),
                Term::Pointer(inner) | Term::ComptimeFunction(inner) => pending.push(inner),
                Term::Function { parameters, result } => {
                    pending.extend(parameters.iter().map(|p| &p.ty));
                    pending.push(result);
                }
                Term::Closure {
                    environment,
                    signature,
                } => {
                    pending.extend(environment);
                    pending.push(signature);
                }
                _ => {}
            }
        }
        Ok(())
    }
}
impl fmt::Display for Package {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(iir-package (roots")?;
        for root in &self.roots {
            write!(f, " {root}")?;
        }
        f.write_str(") (runtime-bindings")?;
        for binding in &self.runtime_bindings {
            write!(f, " (runtime {} {})", binding.instance, binding.local)?;
        }
        f.write_str(")")?;
        if !self.expansion_sources.is_empty() {
            f.write_str(" (expansion-sources")?;
            for (index, origin) in self.expansion_sources.iter().enumerate() {
                write!(f, " (expansion-source {index} {origin})")?;
            }
            f.write_str(")")?;
        }
        f.write_str(" (normal-form-heads")?;
        for head in &self.normal_forms {
            write!(f, " {head}")?;
        }
        f.write_str(") (nominals")?;
        for nominal in &self.nominals {
            write!(f, "\n {nominal}")?;
        }
        f.write_str(") (interfaces")?;
        for interface in &self.interfaces {
            write!(f, "\n {interface}")?;
        }
        f.write_str(") (functions")?;
        for function in &self.functions {
            write!(f, "\n {function}")?;
        }
        f.write_str(") (dispatch")?;
        for instance in &self.dispatch {
            write!(f, "\n {instance}")?;
        }
        f.write_str("))\n")
    }
}
/// Check a completed type/semantic argument. Runtime identities are preserved;
/// missing environments and unsubstituted definition rigids are rejected.
pub fn completed_type(
    ty: &Term,
    available: &BTreeSet<RuntimeId>,
    fuel: &mut usize,
) -> Result<(), String> {
    ty.check_scope(&[])
        .map_err(|e| format!("IIR type scope: {e}"))?;
    completed_dependencies(ty, available, fuel)
}
/// Check dependencies within a term whose outer binder scope was already
/// validated. Nested function parameters/results retain their binder context.
pub fn completed_dependencies(
    ty: &Term,
    available: &BTreeSet<RuntimeId>,
    fuel: &mut usize,
) -> Result<(), String> {
    let mut pending = vec![ty];
    while let Some(ty) = pending.pop() {
        if *fuel == 0 {
            return Err("IIR type validation budget exhausted".into());
        }
        *fuel -= 1;
        match ty {
            Term::Computation {effects,result} => {
                    pending.push(result);
                    for operation in effects {pending.extend(&operation.arguments);}
                }
            Term::AssociatedType { .. } => {
                return Err("IIR type has an unresolved associated type projection".into());
            }
            Term::Value { .. } => {
                return Err("IIR value identity lacks retained value-store evidence".into());
            }
            Term::TypeListConcat(_) => return Err("IIR type retains an unresolved type-list concatenation".into()),
            Term::Rigid { .. } | Term::Hole { .. } => {
                return Err(
                    "IIR type has an unresolved declaration parameter or template hole".into(),
                );
            }
            Term::Runtime(id) if !available.contains(id) => {
                return Err(format!(
                    "IIR type has an unavailable runtime binding {}:{}",
                    id.instance, id.local
                ));
            }
            Term::Nominal { arguments, .. }
            | Term::NormalForm { arguments, .. }
            | Term::Tuple(arguments)
            | Term::TypeListValue(arguments) | Term::TupleValue(arguments) => pending.extend(arguments),
            Term::NominalValue { ty, fields, .. } => { pending.push(ty); pending.extend(fields); },
            Term::Record(fields) | Term::RecordValue(fields) => pending.extend(fields.iter().map(|(_, ty)| ty)),
            Term::Pointer(inner) | Term::ComptimeFunction(inner) => pending.push(inner),
            Term::Function { parameters, result } => {
                pending.extend(parameters.iter().map(|p| &p.ty));
                pending.push(result);
            }
            Term::Closure {
                environment,
                signature,
            } => {
                pending.extend(environment);
                pending.push(signature);
            }
            _ => {}
        }
    }
    Ok(())
}

pub mod cleanup;
