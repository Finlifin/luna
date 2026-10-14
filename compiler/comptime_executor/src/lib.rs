//! Bounded memoization for already-authorized pure comptime queries.
//!
//! The driver owns query identity, dependency revisions, phase authorization and
//! the selected comptime Eq/Hash implementations. This module never substitutes
//! Rust equality of source values for those protocols. Keys/results must be
//! completed immutable values: interpreter storage and live inference state do
//! not belong in this cache.
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct QueryId {
    pub package: u64,
    pub declaration: u32,
    pub operation: u32,
}

/// Changing any dependency world invalidates all entries in this cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scope {
    pub definitions: u64,
    pub protocols: u64,
    pub target: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error<E> {
    BudgetExhausted,
    ReentrantKeyFormation,
    CapacityExceeded,
    Protocol(E),
    Evaluation(E),
}

impl fmt::Display for QueryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(comptime-query (package {}) (declaration {}) (operation {}))",
            self.package, self.declaration, self.operation
        )
    }
}
impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(query-scope (definitions {}) (protocols {}) (target {}))",
            self.definitions, self.protocols, self.target
        )
    }
}
impl<E: fmt::Display> fmt::Display for Error<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BudgetExhausted => f.write_str("comptime query budget exhausted"),
            Self::ReentrantKeyFormation => {
                f.write_str("Eq/Hash key formation requires uncached execution")
            }
            Self::CapacityExceeded => f.write_str("comptime query cache capacity exhausted"),
            Self::Protocol(error) => write!(f, "comptime key protocol failed: {error}"),
            Self::Evaluation(error) => write!(f, "comptime query evaluation failed: {error}"),
        }
    }
}
impl<E: fmt::Debug + fmt::Display> std::error::Error for Error<E> {}

/// Shared by nested queries and their Eq/Hash invocations. Protocol invocations
/// must use the uncached executor while a key is being formed; recursively
/// querying a cache here would require the very key being constructed.
pub struct Context {
    remaining: usize,
    forming_key: bool,
}
impl Context {
    pub fn new(steps: usize) -> Self {
        Self {
            remaining: steps,
            forming_key: false,
        }
    }
    pub fn remaining(&self) -> usize {
        self.remaining
    }
    pub fn spend<E>(&mut self, steps: usize) -> Result<(), Error<E>> {
        self.remaining = self
            .remaining
            .checked_sub(steps)
            .ok_or(Error::BudgetExhausted)?;
        Ok(())
    }
}

/// A driver-selected, phase-checked semantic implementation, fixed for one Scope.
/// These operations must charge their execution work to the supplied context.
/// Equal inputs must have equal hashes. Hash collisions need not imply equality.
/// The caller may only memoize computations compatible with this equivalence.
pub trait KeyProtocol<V> {
    type Error;
    fn hash(&mut self, value: &V, context: &mut Context) -> Result<u64, Error<Self::Error>>;
    fn equivalent(
        &mut self,
        left: &V,
        right: &V,
        context: &mut Context,
    ) -> Result<bool, Error<Self::Error>>;
}

struct Entry<V, R> {
    inputs: Vec<V>,
    result: Arc<R>,
}

pub struct Outcome<R> {
    pub value: Arc<R>,
    pub reused: bool,
}

/// This cache is not an execution authorization boundary. `query_pure` requires
/// the driver's pure promise and complete query dependencies. Impure invocation
/// must go directly to the executor, even when its result supports Eq/Hash.
pub struct Cache<V, R> {
    scope: Scope,
    entries: HashMap<(QueryId, Vec<u64>), Vec<Entry<V, R>>>,
    count: usize,
    capacity: usize,
}
impl<V, R> Cache<V, R> {
    pub fn new(scope: Scope, capacity: usize) -> Self {
        Self {
            scope,
            entries: HashMap::new(),
            count: 0,
            capacity,
        }
    }
    pub fn len(&self) -> usize {
        self.count
    }
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
    pub fn scope(&self) -> Scope {
        self.scope
    }
    pub fn set_scope(&mut self, scope: Scope) {
        if self.scope != scope {
            self.entries.clear();
            self.count = 0;
            self.scope = scope;
        }
    }

    pub fn query_pure<P: KeyProtocol<V>>(
        &mut self,
        query: QueryId,
        inputs: Vec<V>,
        context: &mut Context,
        protocol: &mut P,
        evaluate: impl FnOnce(&[V], &mut Context) -> Result<R, P::Error>,
    ) -> Result<Outcome<R>, Error<P::Error>> {
        self.query(
            query,
            inputs,
            context,
            protocol,
            false,
            |inputs, context| evaluate(inputs, context).map(|value| (value, true)),
        )
    }

    /// A successful computation may decline publication when its result cannot
    /// be frozen. It still returns the result, but later calls must execute again.
    /// At capacity, misses execute without publication; existing hits still work.
    pub fn query_pure_admitted<P: KeyProtocol<V>>(
        &mut self,
        query: QueryId,
        inputs: Vec<V>,
        context: &mut Context,
        protocol: &mut P,
        evaluate: impl FnOnce(&[V], &mut Context) -> Result<(R, bool), P::Error>,
    ) -> Result<Outcome<R>, Error<P::Error>> {
        self.query(query, inputs, context, protocol, true, evaluate)
    }
    fn query<P: KeyProtocol<V>>(
        &mut self,
        query: QueryId,
        inputs: Vec<V>,
        context: &mut Context,
        protocol: &mut P,
        allow_uncached: bool,
        evaluate: impl FnOnce(&[V], &mut Context) -> Result<(R, bool), P::Error>,
    ) -> Result<Outcome<R>, Error<P::Error>> {
        if context.forming_key {
            return Err(Error::ReentrantKeyFormation);
        }
        context.spend(inputs.len().saturating_add(1))?;
        context.forming_key = true;
        // Restore the guard for every ordinary protocol failure. No cache entry
        // is published until hashing, collision comparison and evaluation finish.
        let lookup = (|| {
            let hashes = inputs
                .iter()
                .map(|value| protocol.hash(value, context))
                .collect::<Result<Vec<_>, _>>()?;
            if let Some(bucket) = self.entries.get(&(query, hashes.clone())) {
                for entry in bucket {
                    context.spend(1)?;
                    let mut equal = entry.inputs.len() == inputs.len();
                    for (stored, incoming) in entry.inputs.iter().zip(&inputs) {
                        context.spend(1)?;
                        if !protocol.equivalent(stored, incoming, context)? {
                            equal = false;
                            break;
                        }
                    }
                    if equal {
                        return Ok((hashes, Some(Arc::clone(&entry.result))));
                    }
                }
            }
            Ok((hashes, None))
        })();
        context.forming_key = false;
        let (hashes, found) = lookup?;
        if let Some(value) = found {
            return Ok(Outcome {
                value,
                reused: true,
            });
        }
        if self.count >= self.capacity && !allow_uncached {
            return Err(Error::CapacityExceeded);
        }
        context.spend(1)?;
        let (value, publish) = evaluate(&inputs, context).map_err(Error::Evaluation)?;
        let value = Arc::new(value);
        if !publish || self.count >= self.capacity {
            return Ok(Outcome {
                value,
                reused: false,
            });
        }
        self.entries
            .entry((query, hashes))
            .or_default()
            .push(Entry {
                inputs,
                result: Arc::clone(&value),
            });
        self.count += 1;
        Ok(Outcome {
            value,
            reused: false,
        })
    }
}
