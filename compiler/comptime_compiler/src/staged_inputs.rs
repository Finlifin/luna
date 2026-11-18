//! Code for evaluating contextual operands before their domain is available.
//! Literal and container instructions carry their source operand path.
use super::*;

#[derive(Debug)]
pub enum Instruction {
    Enter,
    Leave,
    Literal {
        value: Register,
        path: nir::program::InputPath,
    },
    Unit(Register),
    Append {
        parent: Register,
        value: Register,
        spread: bool,
        diverges: bool,
    },
    Container {
        path: nir::program::InputPath,
        value: Register,
        list: bool,
        diverges: bool,
    },
    Transfer {
        destination: Register,
        value: Register,
        diverges: bool,
    },
    Finish {
        value: Register,
        diverges: bool,
    },
}
impl fmt::Display for Instruction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Enter => write!(f, "(stage-enter)"),
            Self::Leave => write!(f, "(stage-leave)"),
            Self::Unit(slot) => write!(f, "(stage-unit %{slot})"),
            Self::Literal { value, path } => write!(f, "(stage-literal %{value} {path})"),
            Self::Append {
                parent,
                value,
                spread,
                diverges,
            } => write!(
                f,
                "(stage-append %{parent} %{value} (spread {spread}) (diverges {diverges}))"
            ),
            Self::Container {
                path,
                value,
                list,
                diverges,
            } => write!(
                f,
                "(stage-container %{value} {path} (list {list}) (diverges {diverges}))"
            ),
            Self::Transfer {
                destination,
                value,
                diverges,
            } => write!(
                f,
                "(stage-transfer %{destination} %{value} (diverges {diverges}))"
            ),
            Self::Finish { value, diverges } => {
                write!(f, "(stage-finish %{value} (diverges {diverges}))")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nir::program::NormalFormArgument as A;
    const SOURCE: SourceRange = SourceRange { start: 1, end: 2 };

    #[test]
    fn staged_code_owns_literals_and_bounds_all_compilation_work() {
        let input = A::List {
            source: SOURCE,
            items: vec![
                A::Integer {
                    source: SOURCE,
                    spelling: "17".into(),
                    negative: false,
                },
                A::Tuple {
                    source: SOURCE,
                    items: vec![A::Unit { source: SOURCE }],
                },
            ],
        };
        let (code, shape) = compile_staged_input(&input, &mut 10000).unwrap();
        let work = code.storage_work();
        for mut fuel in 0..work {
            assert!(matches!(
                compile_staged_input(&input, &mut fuel),
                Err(Error::Budget(_))
            ));
        }
        let mut exact = work;
        assert!(compile_staged_input(&input, &mut exact).is_ok());
        let dump = code.to_string();
        assert!(dump.contains("(stage-literal "));
        assert!(dump.contains("(stage-append "));
        assert!(dump.contains("(stage-container "));
        assert!(dump.contains("(stage-finish "));
        assert!(!dump.contains("(prepare-normal-input "));
        assert_eq!(
            dump,
            compile_staged_input(&input, &mut 10000)
                .unwrap()
                .0
                .to_string()
        );
        drop(input);
        assert!(shape.to_string().contains("\"17\""));
        assert_eq!(code.to_string(), dump);
    }
}
