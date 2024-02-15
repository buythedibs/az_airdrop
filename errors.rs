use ink::{
    env::Error as InkEnvError,
    prelude::{format, string::String},
    LangError,
};
#[derive(Debug, PartialEq, Eq, scale::Encode, scale::Decode)]
#[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
pub enum AzAirdropError {
    ContractCall(LangError),
    InkEnvError(String),
    NotFound(String),
    Unauthorised,
    UnprocessableEntity(String),
}
impl From<InkEnvError> for AzAirdropError {
    fn from(e: InkEnvError) -> Self {
        AzAirdropError::InkEnvError(format!("{e:?}"))
    }
}
impl From<LangError> for AzAirdropError {
    fn from(e: LangError) -> Self {
        AzAirdropError::ContractCall(e)
    }
}
