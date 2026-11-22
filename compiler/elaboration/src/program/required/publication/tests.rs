use super::*;
use std::sync::Arc;

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 7, local }
}

fn record(local: u32) -> n::Struct {
    n::Struct {
        declaration: id(local),
        name: format!("Record{local}"),
        type_parameters: vec![],
        parameter_types: vec![],
        capabilities: vec![],
        requirements: vec![],
        fields: vec![("flag".into(), p(P::Bool))],
        packed: nir::program::PackedMode::None,
    }
}

fn generated() -> PublishedDefinitions {
    PublishedDefinitions {
        // Separate batches exercise failure after an earlier batch was ready.
        structs: vec![Arc::from([record(10)]), Arc::from([record(11)])],
        enums: vec![Arc::from([n::Enum {
            declaration: id(12),
            name: "Choice".into(),
            is_intrinsic: false,
            parameter_types: vec![],
            capabilities: vec![],
            requirements: vec![],
            type_parameters: vec![],
            variants: vec![n::Variant {
                name: "some".into(),
                shape: n::VariantShape::Tuple,
                fields: vec![Term::Nominal {
                    declaration: id(10),
                    arguments: vec![],
                }],
            }],
            groups: vec![],
            encoding: None,
            symbol_encoding: None,
        }])],
        newtypes: vec![Arc::from([nir::newtypes::Newtype {
            declaration: id(13),
            name: "Wrapped".into(),
            type_parameters: vec![],
            parameter_types: vec![],
            capture_count: 0,
            requirements: vec![],
            capabilities: vec![],
            representation: Term::Nominal {
                declaration: id(12),
                arguments: vec![],
            },
        }])],
    }
}

#[derive(Default)]
struct World {
    context: crate::ptc::Context,
    declared_nominals: crate::ptc::Context,
    structs: BTreeMap<DeclarationId, n::Struct>,
    enums: BTreeMap<DeclarationId, n::Enum>,
    newtypes: BTreeMap<DeclarationId, nir::newtypes::Newtype>,
    index: crate::implementation::Index,
}

impl World {
    fn register(
        &mut self,
        definitions: &PublishedDefinitions,
        steps: &mut u64,
    ) -> Result<(), Error> {
        Receiver {
            context: &mut self.context,
            declared_nominals: &mut self.declared_nominals,
            structs: &mut self.structs,
            enums: &mut self.enums,
            newtypes: &mut self.newtypes,
            implementation_index: &mut self.index,
        }
        .register(definitions, steps, Span::default())
    }

    fn seeded() -> Self {
        let mut world = Self::default();
        world
            .register(
                &PublishedDefinitions {
                    structs: vec![Arc::from([record(1)])],
                    ..Default::default()
                },
                &mut 100_000,
            )
            .unwrap();
        world
    }

    fn assert_seed_only(&self, revision: u64, index: &str) {
        assert_eq!(self.structs, BTreeMap::from([(id(1), record(1))]));
        assert!(self.enums.is_empty());
        assert!(self.newtypes.is_empty());
        assert!(self.context.is_nominal_constructor(id(1)));
        assert!(self.declared_nominals.is_nominal_constructor(id(1)));
        assert_eq!(self.context.formed_nullary_nominals().len(), 1);
        assert_eq!(self.declared_nominals.formed_nullary_nominals().len(), 1);
        for local in 10..=13 {
            assert!(!self.declared_nominals.is_nominal_constructor(id(local)));
            assert!(
                !self.context.is_nominal_constructor(id(local)),
                "leaked constructor {local}"
            );
        }
        assert_eq!(self.index.revision(), revision);
        assert_eq!(self.index.to_string(), index);
    }

    fn assert_complete(&self, definitions: &PublishedDefinitions) {
        for ctx in [&self.context, &self.declared_nominals] {
            let formed = ctx.formed_nullary_nominals();
            assert_eq!(
                formed.of_kind(nir::NominalKind::Struct).collect::<Vec<_>>(),
                [id(1), id(10), id(11)]
            );
            assert!(formed.contains(nir::NominalKind::Enum, id(12)));
            assert!(formed.contains(nir::NominalKind::Newtype, id(13)));
        }
        assert_eq!(self.structs.len(), 3);
        assert_eq!(self.enums.len(), 1);
        assert_eq!(self.newtypes.len(), 1);
        for batch in &definitions.structs {
            for definition in batch.iter() {
                assert_eq!(self.structs.get(&definition.declaration), Some(definition));
            }
        }
        assert_eq!(self.enums.get(&id(12)), Some(&definitions.enums[0][0]));
        assert_eq!(
            self.newtypes.get(&id(13)),
            Some(&definitions.newtypes[0][0])
        );
        for local in [1, 10, 11, 12, 13] {
            assert!(self.context.is_nominal_constructor(id(local)));
            assert!(self.declared_nominals.is_nominal_constructor(id(local)));
        }
        // Synchronizing the same complete layouts must not change evidence
        // revision: the production receiver already delivered every kind.
        let mut expected = crate::implementation::Index::default();
        expected.sync_nominal_layouts(
            self.structs.values(),
            self.enums.values(),
            self.newtypes.values(),
        );
        assert_eq!(self.index.nominal_layouts(), expected.nominal_layouts());
    }
}

#[test]
fn exhausted_registration_keeps_all_definition_kinds_and_evidence_private() {
    let definitions = generated();
    let mut successful = World::seeded();
    let mut fuel = 100_000;
    successful.register(&definitions, &mut fuel).unwrap();
    let cost = 100_000 - fuel;
    assert!(cost > 0);
    successful.assert_complete(&definitions);

    for allowance in 0..cost {
        let mut world = World::seeded();
        let revision = world.index.revision();
        let index = world.index.to_string();
        let mut remaining = allowance;
        let failure = world.register(&definitions, &mut remaining).unwrap_err();
        assert!(
            failure.message.contains("registration budget exhausted"),
            "{failure}"
        );
        assert!(remaining <= allowance);
        world.assert_seed_only(revision, &index);

        // A fresh receiver attempt can use the same immutable descriptors,
        // without rerunning the builder or allocating replacement identities.
        let mut retry = cost;
        world.register(&definitions, &mut retry).unwrap();
        assert_eq!(retry, 0);
        world.assert_complete(&definitions);
    }
}

#[test]
fn empty_registration_is_a_noop_even_without_budget() {
    let mut world = World::seeded();
    let revision = world.index.revision();
    let index = world.index.to_string();
    world
        .register(&PublishedDefinitions::default(), &mut 0)
        .unwrap();
    world.assert_seed_only(revision, &index);
}
