//! Rebuild structural values in postorder. The only nested traversal borrows a
//! locked storage payload. Its guard stays live until that payload is restored.
use super::*;
use comptime_runtime::object::Entry;

enum Group<'v> {
    Tuple,
    Array(Term),
    List(Term),
    Record(DeclarationId, Vec<Term>),
    Variant(DeclarationId, Vec<Term>, usize),
    Anonymous(&'v [(String, Value)]),
    Object(&'v [Entry<ObjectValue>]),
    Optional(Term),
    Newtype(Term),
    Error(Term, Option<usize>),
    Closure(&'v Arc<Closure>, Arc<Expr>),
}

enum Work<'v> {
    Read(&'v Value, usize),
    Named(&'v str, &'v Value, usize),
    Entry(&'v Entry<ObjectValue>, usize),
    Key(&'v str),
    Finish(Group<'v>, usize),
}

#[derive(Default)]
struct Packet<'v> {
    work: Vec<Work<'v>>,
    values: Vec<Value>,
    entry_types: Vec<Term>,
}

enum Progress<'v> {
    Cell(&'v Storage, usize),
    Complete(Value),
}

impl Restore<'_> {
    pub(super) fn value(&mut self, value: &Value, depth: usize) -> Result<Value, ErrorKind> {
        let mut packet = Packet::default();
        packet.work.push(Work::Read(value, depth));
        loop {
            match packet.advance(self)? {
                Progress::Complete(result) => return Ok(result),
                Progress::Cell(cell, depth) => {
                    let payload = cell.contents_for_copy(self.steps, 128 - depth)?;
                    let restored = Storage::new(self.value(payload.value(), depth + 1)?);
                    self.storage
                        .insert(Arc::as_ptr(&cell.0) as usize, Some(restored.clone()));
                    packet.values.push(Value::Storage(restored));
                }
            }
        }
    }
}

impl<'v> Packet<'v> {
    fn group(&mut self, group: Group<'v>, fields: impl IntoIterator<Item = Work<'v>>) {
        self.work.push(Work::Finish(group, self.values.len()));
        let base = self.work.len();
        self.work.extend(fields);
        self.work[base..].reverse();
    }

    fn advance(&mut self, restore: &mut Restore<'_>) -> Result<Progress<'v>, ErrorKind> {
        while let Some(work) = self.work.pop() {
            let (value, depth) = match work {
                Work::Read(value, depth) => (value, depth),
                Work::Named(name, value, depth) => {
                    restore.spend(name.len())?;
                    (value, depth)
                }
                Work::Entry(entry, depth) => {
                    let field = entry.value();
                    self.entry_types.push(restore.ty(&field.ty)?);
                    if let Some(key) = entry.key() {
                        self.work.push(Work::Key(key));
                    }
                    (&field.value, depth)
                }
                Work::Key(key) => {
                    restore.spend(key.len())?;
                    continue;
                }
                Work::Finish(group, base) => {
                    let fields = self.values.split_off(base);
                    let value = group.finish(restore, fields, &mut self.entry_types)?;
                    self.values.push(value);
                    continue;
                }
            };
            restore.spend(1)?;
            if depth >= 128 {
                return Err(ErrorKind::DepthExhausted);
            }
            let child = |value| Work::Read(value, depth + 1);
            match value {
                Value::Tuple(fields) => {
                    restore.spend(fields.len())?;
                    self.group(Group::Tuple, fields.iter().map(child));
                }
                Value::Array { ty, values } => {
                    let ty = restore.ty(ty)?;
                    restore.spend(values.len())?;
                    self.group(Group::Array(ty), values.iter().map(child));
                }
                Value::List { ty, values } => {
                    let ty = restore.ty(ty)?;
                    restore.spend(values.len())?;
                    self.group(Group::List(ty), values.iter().map(child));
                }
                Value::Record {
                    declaration,
                    arguments,
                    fields,
                } => {
                    let arguments = restore.types(arguments)?;
                    restore.spend(fields.len())?;
                    self.group(
                        Group::Record(*declaration, arguments),
                        fields.iter().map(child),
                    );
                }
                Value::Variant {
                    declaration,
                    arguments,
                    index,
                    fields,
                } => {
                    let arguments = restore.types(arguments)?;
                    restore.spend(fields.len())?;
                    self.group(
                        Group::Variant(*declaration, arguments, *index),
                        fields.iter().map(child),
                    );
                }
                Value::AnonymousRecord(fields) => {
                    restore.spend(fields.len())?;
                    self.group(
                        Group::Anonymous(fields),
                        fields
                            .iter()
                            .map(|(name, value)| Work::Named(name, value, depth + 1)),
                    );
                }
                Value::Object(object) => {
                    let entries = object.entries();
                    restore.spend(entries.len())?;
                    self.group(
                        Group::Object(entries),
                        entries.iter().map(|entry| Work::Entry(entry, depth + 1)),
                    );
                }
                Value::Optional { ty, value } => self.group(
                    Group::Optional(restore.ty(ty)?),
                    value.as_deref().map(child),
                ),
                Value::Newtype { ty, value } => {
                    self.group(Group::Newtype(restore.ty(ty)?), [child(value)])
                }
                Value::ErrorResult { ty, error, value } => {
                    self.group(Group::Error(restore.ty(ty)?, *error), [child(value)])
                }
                Value::Storage(cell) => {
                    let key = Arc::as_ptr(&cell.0) as usize;
                    if let Some(saved) = restore.storage.get(&key) {
                        self.values
                            .push(Value::Storage(saved.clone().ok_or(ErrorKind::Unsupported)?));
                    } else {
                        restore.storage.insert(key, None);
                        return Ok(Progress::Cell(cell, depth));
                    }
                }
                Value::Closure(closure) => {
                    let key = Arc::as_ptr(closure) as usize;
                    if let Some(saved) = restore.closures.get(&key) {
                        self.values
                            .push(Value::Closure(saved.clone().ok_or(ErrorKind::Unsupported)?));
                        continue;
                    }
                    restore.closures.insert(key, None);
                    let body = restore.body(&closure.body)?;
                    restore.spend(
                        closure.parameters.len()
                            + closure.capture_order.len()
                            + closure.environment.len(),
                    )?;
                    self.group(
                        Group::Closure(closure, body),
                        closure.environment.values().map(child),
                    );
                }
                value => self.values.push(restore.leaf(value)?),
            }
        }
        Ok(Progress::Complete(
            self.values.pop().expect("restored packet root"),
        ))
    }
}

impl Group<'_> {
    fn finish(
        self,
        restore: &mut Restore<'_>,
        mut fields: Vec<Value>,
        entry_types: &mut Vec<Term>,
    ) -> Result<Value, ErrorKind> {
        Ok(match self {
            Self::Tuple => Value::Tuple(fields),
            Self::Array(ty) => Value::Array { ty, values: fields },
            Self::List(ty) => Value::List {
                ty,
                values: fields.into(),
            },
            Self::Record(declaration, arguments) => Value::Record {
                declaration,
                arguments,
                fields,
            },
            Self::Variant(declaration, arguments, index) => Value::Variant {
                declaration,
                arguments,
                index,
                fields,
            },
            Self::Anonymous(original) => Value::AnonymousRecord(
                original
                    .iter()
                    .map(|(name, _)| name.clone())
                    .zip(fields)
                    .collect(),
            ),
            Self::Object(entries) => {
                let types = entry_types.drain(entry_types.len() - fields.len()..);
                let entries = entries
                    .iter()
                    .zip(types.zip(fields))
                    .map(|(entry, (ty, value))| {
                        let field = ObjectValue { ty, value };
                        match entry.key() {
                            Some(key) => Entry::Property {
                                key: key.to_owned(),
                                value: field,
                            },
                            None => Entry::Value(field),
                        }
                    })
                    .collect();
                Value::Object(Object::from_entries(entries))
            }
            Self::Optional(ty) => Value::Optional {
                ty,
                value: fields.pop().map(Box::new),
            },
            Self::Newtype(ty) => Value::Newtype {
                ty,
                value: Box::new(fields.pop().expect("newtype payload")),
            },
            Self::Error(ty, error) => Value::ErrorResult {
                ty,
                error,
                value: Box::new(fields.pop().expect("result payload")),
            },
            Self::Closure(closure, body) => {
                let environment = closure
                    .environment
                    .iter()
                    .map(|(id, _)| *id)
                    .zip(fields)
                    .collect();
                let restored = Arc::new(Closure {
                    definition_source: closure.definition_source,
                    parameters: closure.parameters.clone(),
                    capture_order: closure.capture_order.clone(),
                    environment,
                    body,
                    ty: restore.ty(&closure.ty)?,
                });
                restore
                    .closures
                    .insert(Arc::as_ptr(closure) as usize, Some(restored.clone()));
                Value::Closure(restored)
            }
        })
    }
}
