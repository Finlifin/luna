//! Uncached source Eq/Hash execution for one concrete input type. The owner of a
//! query cache still establishes immutability, purity and equality-compatible
//! reuse. Typed immutable snapshots do not enable global memoization.
use comptime_executor::{Context, Error, KeyProtocol};
use comptime_interpretor::{Budget, ContractOutcome, Interpreter, Value};
use elaboration::{
    capabilities::World,
    implementation::{ImplementationHead, Index},
    key_protocols::{Operation, Selected},
    ptc,
};
use intrinsic::LangItems;
use nir::{
    Term,
    program::{Package, TypePredicate},
};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// A checked immutable value belonging to one prepared protocol world. No
/// structural Eq/Hash implementation: source protocols retain that authority.
#[derive(Clone)]
pub struct Snapshot {
    value: Arc<Value>,
    ty: Arc<Term>,
    owner: Arc<()>,
}
impl Snapshot {
    pub fn value(&self) -> &Value {
        &self.value
    }
    pub fn ty(&self) -> &Term {
        &self.ty
    }
}
impl std::fmt::Display for Snapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(comptime-snapshot (type {}) (value {}))",
            self.ty, self.value
        )
    }
}

pub struct Protocol {
    owner: Arc<()>,
    subject: Arc<Term>,
    package: Package,
    context: ptc::Context,
    index: Index,
    eq: Selected,
    hash: Selected,
    budget: ptc::Budget,
}
impl Protocol {
    /// Bind both operations, prove type capabilities, and prepare their
    /// transitive dispatch dependencies transactionally in a private package.
    /// The supplied package must have passed ordinary source contract checking.
    pub fn prepare(
        package: &Package,
        items: &LangItems,
        subject: &Term,
        budget: ptc::Budget,
    ) -> Result<Self, String> {
        let context = ptc::Context::from_package(package)?;
        let mut index = Index::default();
        for i in package
            .implementations
            .iter()
            .filter(|i| i.interface.is_some())
        {
            index
                .insert(
                    ImplementationHead {
                        parameter_types: i.parameter_types.clone(),
                        declaration: i.declaration,
                        interface: i.interface,
                        interface_arguments: i.interface_arguments.clone(),
                        subject: i.subject.clone(),
                        type_parameters: i.type_parameters.clone(),
                        requirements: i.requirements.clone(),
                    },
                    budget,
                )
                .map_err(|e| format!("key implementation index: {e:?}"))?;
        }
        let world = World {
            context: &context,
            index: &index,
            implementations: &package.implementations,
            traits: &package.traits,
            assumptions: &[],
        };
        let eq = crate::protocols::select_comptime_key(
            package,
            &world,
            items,
            subject,
            Operation::Eq,
            budget,
        )?;
        let hash = crate::protocols::select_comptime_key(
            package,
            &world,
            items,
            subject,
            Operation::Hash,
            budget,
        )?;
        for method in [&eq, &hash] {
            for capability in &method.capabilities {
                if !matches!(
                    world.check_predicate(&TypePredicate::Trait(capability.clone()), budget)?,
                    ptc::Outcome::Entailed(_)
                ) {
                    return Err("comptime key capability is not established".into());
                }
            }
        }
        let roots = [&eq, &hash].map(|m| iir::InstanceKey {
            declaration: m.method.body,
            arguments: m.method.arguments.clone(),
        });
        let dispatch =
            elaboration::materialization::dispatch_instances_for_roots(package, &roots, budget)?;
        let mut package = package.clone();
        package.dispatch_instances = dispatch;
        Ok(Self {
            owner: Arc::new(()),
            subject: Arc::new(subject.clone()),
            package,
            context,
            index,
            eq,
            hash,
            budget,
        })
    }
    fn invoke(
        &self,
        method: &Selected,
        inputs: &[&Value],
        context: &mut Context,
    ) -> Result<Value, Error<String>> {
        // Reject host references/callables and account for copying before
        // cloning inputs into the fresh invocation. This is not value freezing.
        for value in inputs {
            input_work(value, context)?;
        }
        self.run(context, |interpreter| {
            for value in inputs {
                interpreter.validate_immutable_input(value, &self.subject)?;
            }
            interpreter.invoke_instantiated(
                method.method.body,
                method.method.arguments.clone(),
                inputs.iter().map(|v| (*v).clone()).collect(),
            )
        })
    }
    /// Validate and own immutable data without executing Eq or Hash. References
    /// and captured functions are not frozen by copying their host handles.
    pub fn admit(&self, value: Value, context: &mut Context) -> Result<Snapshot, Error<String>> {
        self.run(context, |interpreter| {
            interpreter.validate_immutable_input(&value, &self.subject)
        })?;
        Ok(Snapshot {
            value: Arc::new(value),
            ty: self.subject.clone(),
            owner: self.owner.clone(),
        })
    }
    fn run<R>(
        &self,
        context: &mut Context,
        action: impl FnOnce(&mut Interpreter<'_>) -> Result<R, comptime_interpretor::Error>,
    ) -> Result<R, Error<String>> {
        let world = World {
            context: &self.context,
            index: &self.index,
            implementations: &self.package.implementations,
            traits: &self.package.traits,
            assumptions: &[],
        };
        let solver = |predicate: &TypePredicate| match world.check_predicate(predicate, self.budget)
        {
            Ok(ptc::Outcome::Entailed(_)) => ContractOutcome::Satisfied,
            Ok(ptc::Outcome::NotEntailed(_)) => ContractOutcome::Rejected,
            _ => ContractOutcome::Unknown,
        };
        let steps = u64::try_from(context.remaining()).unwrap_or(u64::MAX);
        let mut interpreter =
            Interpreter::for_comptime(&self.package, Budget { steps, depth: 128 })
                .map_err(|e| Error::Protocol(e.to_string()))?
                .with_type_solver(&solver);
        let result = action(&mut interpreter);
        context.spend::<String>((steps - interpreter.remaining_steps()) as usize)?;
        result.map_err(|e| Error::Protocol(e.to_string()))
    }
}
impl KeyProtocol<Value> for Protocol {
    type Error = String;
    fn hash(&mut self, value: &Value, context: &mut Context) -> Result<u64, Error<String>> {
        let value = self.invoke(&self.hash, &[value], context)?;
        let mut bucket = std::collections::hash_map::DefaultHasher::new();
        match value {
            Value::Integer(value) => {
                context.spend::<String>(value.bit_len().div_ceil(32).saturating_add(1))?;
                value.hash(&mut bucket);
            }
            Value::SignedInteger(value) => value.hash(&mut bucket),
            Value::UnsignedInteger(value) => value.hash(&mut bucket),
            _ => return Err(Error::Protocol("Hash returned a non-integer value".into())),
        }
        // This digest only chooses a bucket. Semantic Eq always decides a hit.
        Ok(bucket.finish())
    }
    fn equivalent(
        &mut self,
        a: &Value,
        b: &Value,
        context: &mut Context,
    ) -> Result<bool, Error<String>> {
        match self.invoke(&self.eq, &[a, b], context)? {
            Value::Bool(value) => Ok(value),
            _ => Err(Error::Protocol("Eq returned a non-Boolean value".into())),
        }
    }
}
impl KeyProtocol<Snapshot> for Protocol {
    type Error = String;
    fn hash(&mut self, value: &Snapshot, context: &mut Context) -> Result<u64, Error<String>> {
        self.check_owner(value)?;
        <Self as KeyProtocol<Value>>::hash(self, value.value(), context)
    }
    fn equivalent(
        &mut self,
        a: &Snapshot,
        b: &Snapshot,
        context: &mut Context,
    ) -> Result<bool, Error<String>> {
        self.check_owner(a)?;
        self.check_owner(b)?;
        <Self as KeyProtocol<Value>>::equivalent(self, a.value(), b.value(), context)
    }
}
impl Protocol {
    pub(crate) fn owns(&self, value: &Snapshot) -> bool {
        Arc::ptr_eq(&self.owner, &value.owner)
    }
    fn check_owner(&self, value: &Snapshot) -> Result<(), Error<String>> {
        if self.owns(value) {
            Ok(())
        } else {
            Err(Error::Protocol(
                "snapshot belongs to a different prepared protocol world".into(),
            ))
        }
    }
}
impl std::fmt::Display for Protocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "(comptime-key-protocol {} {})", self.eq, self.hash)
    }
}
fn input_work(value: &Value, context: &mut Context) -> Result<(), Error<String>> {
    let work = value
        .immutable_copy_work()
        .ok_or_else(|| Error::Protocol("key input is not bounded immutable data".into()))?;
    context.spend::<String>(work)
}
