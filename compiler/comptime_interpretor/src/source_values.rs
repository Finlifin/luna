//! Frozen values may refer to nominal layouts published after the caller's
//! snapshot. Admit just their concrete descriptions in the receiving VM.
use super::*;

#[derive(Clone)]
pub enum NominalBatch {
    Structs(Arc<[nir::program::Struct]>),
    Enums(Arc<[nir::program::Enum]>),
    Newtypes(Arc<[nir::newtypes::Newtype]>),
}

impl NominalBatch {
    pub fn len(&self) -> usize {
        match self {
            Self::Structs(batch) => batch.len(),
            Self::Enums(batch) => batch.len(),
            Self::Newtypes(batch) => batch.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn declaration(&self, index: usize) -> DeclarationId {
        match self {
            Self::Structs(batch) => batch[index].declaration,
            Self::Enums(batch) => batch[index].declaration,
            Self::Newtypes(batch) => batch[index].declaration,
        }
    }

    fn install(&self, vm: &mut Interpreter<'_>) -> Result<(), Error> {
        match self {
            Self::Structs(batch) => vm
                .install_owned_generated_structs_with(batch.clone(), || Ok(()))
                .map_err(|e| match e {
                    StructPublicationError::Admission(e) | StructPublicationError::Commit(e) => e,
                }),
            Self::Enums(batch) => vm
                .install_owned_generated_enums_with(batch.clone(), || Ok(()))
                .map_err(|e| match e {
                    EnumPublicationError::Admission(e) | EnumPublicationError::Commit(e) => e,
                }),
            Self::Newtypes(batch) => vm
                .install_owned_generated_newtypes_with(batch.clone(), || Ok(()))
                .map_err(|e| match e {
                    NewtypePublicationError::Admission(e) | NewtypePublicationError::Commit(e) => e,
                }),
        }
    }
}

/// The supplying source owns the checked descriptors and their finite-storage
/// proofs. The payload itself grants no authority to manufacture a declaration.
/// Dependencies precede their users. Descriptors already owned by the caller
/// need no transfer. Every supplied batch still undergoes receiver admission.
pub struct SourcedValue {
    pub value: Arc<Term>,
    pub definitions: Vec<NominalBatch>,
}

impl fmt::Display for SourcedValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(sourced-value (value {}) (imports", self.value)?;
        for batch in &self.definitions {
            let kind = match batch {
                NominalBatch::Structs(_) => "structs",
                NominalBatch::Enums(_) => "enums",
                NominalBatch::Newtypes(_) => "newtypes",
            };
            write!(f, " ({kind}")?;
            for i in 0..batch.len() {
                write!(f, " {}", batch.declaration(i))?;
            }
            f.write_str(")")?;
        }
        f.write_str("))")
    }
}

pub type AttributeSource<'a> = dyn Fn(
        DeclarationId,
        &dyn Fn(DeclarationId) -> bool,
        &mut u64,
    ) -> Result<Option<SourcedValue>, Error>
    + 'a;

pub(super) enum Resolver<'a> {
    Value(&'a AttributeResolver<'a>),
    Source(&'a AttributeSource<'a>),
}

struct Imported<'vm, 'a, 'd> {
    vm: &'vm mut Interpreter<'a>,
    installed: &'d [NominalBatch],
}

impl Drop for Imported<'_, '_, '_> {
    fn drop(&mut self) {
        for batch in self.installed.iter().rev() {
            for i in 0..batch.len() {
                let id = batch.declaration(i);
                match batch {
                    NominalBatch::Structs(_) => {
                        self.vm.structs.remove(&id);
                    }
                    NominalBatch::Enums(_) => {
                        self.vm.enums.remove(&id);
                    }
                    NominalBatch::Newtypes(_) => self.vm.newtypes.remove(&id),
                }
            }
        }
    }
}

impl SourcedValue {
    pub(super) fn decode(&self, vm: &mut Interpreter<'_>, domain: &Term) -> Result<Value, Error> {
        let mut imports = Imported { vm, installed: &[] };
        for (i, batch) in self.definitions.iter().enumerate() {
            imports
                .vm
                .charge_preparation(batch.len().saturating_add(1))?;
            batch.install(imports.vm)?;
            imports.installed = &self.definitions[..=i];
        }
        let value = imports.vm.decode_immutable_input(&self.value, domain)?;
        imports.installed = &[];
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_sourced_values_restore_the_world_and_can_be_retried() {
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let id = DeclarationId {
            package: 3,
            local: 17,
        };
        let ty = Term::Nominal {
            declaration: id,
            arguments: vec![],
        };
        let object = Term::Primitive(Primitive::Object);
        let batch = NominalBatch::Structs(Arc::from([nir::program::Struct {
            declaration: id,
            name: "Generated".into(),
            fields: vec![],
            packed: nir::program::PackedMode::None,
            type_parameters: vec![],
            parameter_types: vec![],
            requirements: vec![],
            capabilities: vec![],
        }]));
        let payload = Arc::new(nir::objects::value(vec![nir::objects::entry(
            Some("witness"),
            Term::Primitive(Primitive::Type),
            ty,
        )]));
        let mut delivery = SourcedValue {
            value: payload.clone(),
            definitions: vec![batch.clone(), batch],
        };
        assert_eq!(
            delivery.decode(&mut vm, &object).unwrap_err().kind,
            ErrorKind::InvalidCallPlan
        );
        assert!(!vm.structs.contains_key(&id));

        delivery.definitions.pop();
        delivery.value = Arc::new(Term::Bool(true));
        assert!(delivery.decode(&mut vm, &object).is_err());
        assert!(!vm.structs.contains_key(&id));

        delivery.value = payload;
        let before = vm.remaining_steps();
        assert!(matches!(
            delivery.decode(&mut vm, &object).unwrap(),
            Value::Object(_)
        ));
        assert!(vm.structs.contains_key(&id));
        let consumed = before - vm.remaining_steps();

        let mut limited = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        limited.budget.steps = consumed - 1;
        assert_eq!(
            delivery.decode(&mut limited, &object).unwrap_err().kind,
            ErrorKind::StepsExhausted
        );
        assert!(!limited.structs.contains_key(&id));
        limited.budget.steps = before;
        assert!(delivery.decode(&mut limited, &object).is_ok());
    }
}
