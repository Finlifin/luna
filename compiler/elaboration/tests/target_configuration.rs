use elaboration::program::{Protocols, elaborate, elaborate_packages_for_target};
use nir::target::TargetData;
use std::collections::BTreeMap;

#[test]
fn explicit_target_is_retained_without_polluting_target_independent_requests() {
    let input = hir::Package::new();
    let independent = elaborate(&input).unwrap();
    assert_eq!(independent.target, None);
    assert!(!independent.to_string().contains("target-data"));
    let mut previous = None;
    for width in [32, 64, 32] {
        let target = TargetData::new(width).unwrap();
        let package = elaborate_packages_for_target(
            &input,
            &BTreeMap::new(),
            &Protocols::default(),
            Some(target),
        )
        .unwrap();
        assert_eq!(package.target, Some(target));
        assert!(package.to_string().contains(&target.to_string()));
        assert_eq!(package.clone(), package);
        let graph = elaboration::materialization::instantiate(
            &package,
            &[],
            Default::default(),
            Default::default(),
        )
        .unwrap();
        assert_eq!(graph.target, package.target);
        assert!(graph.to_string().contains(&target.to_string()));
        if let Some(other) = previous {
            assert_ne!(package, other);
        }
        previous = Some(package);
    }
    assert_eq!(elaborate(&input).unwrap(), independent);
}
