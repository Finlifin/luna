//! Storage-phase failures are independent of type-proposition solving.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageError {
    Budget,
    Invalid(&'static str),
}
impl StorageError {
    pub fn message(self) -> &'static str {
        match self {
            Self::Budget => "storage inspection budget exhausted",
            Self::Invalid(message) => message,
        }
    }
}
impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}
impl std::error::Error for StorageError {}
impl From<crate::type_computations::Error> for StorageError {
    fn from(error: crate::type_computations::Error) -> Self {
        match error {
            crate::type_computations::Error::Budget => Self::Budget,
            _ => Self::Invalid("invalid storage type description"),
        }
    }
}
impl From<crate::NominalAdmissionError> for StorageError {
    fn from(error: crate::NominalAdmissionError) -> Self {
        match error {
            crate::NominalAdmissionError::Budget => Self::Budget,
            _ => Self::Invalid("invalid nominal storage layout"),
        }
    }
}
impl From<crate::newtypes::ConversionError> for StorageError {
    fn from(error: crate::newtypes::ConversionError) -> Self {
        match error {
            crate::newtypes::ConversionError::Budget => Self::Budget,
            _ => Self::Invalid("invalid newtype storage representation"),
        }
    }
}
impl From<StorageError> for String {
    fn from(error: StorageError) -> Self {
        error.to_string()
    }
}
