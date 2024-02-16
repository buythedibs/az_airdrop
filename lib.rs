#![cfg_attr(not(feature = "std"), no_std, no_main)]

mod errors;

#[ink::contract]
mod az_airdrop {
    use crate::errors::AzAirdropError;
    use ink::prelude::vec::Vec;
    use ink::storage::Lazy;
    use ink::{
        env::CallFlags, prelude::string::ToString, reflect::ContractEventBase, storage::Mapping,
    };
    use openbrush::contracts::psp22::PSP22Ref;

    // === TYPES ===
    type Event = <AzAirdrop as ContractEventBase>::Type;
    type Result<T> = core::result::Result<T, AzAirdropError>;

    // === STRUCTS ===
    #[derive(Debug, Clone, scale::Encode, scale::Decode)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub struct Config {
        admin: AccountId,
        sub_admins: Vec<AccountId>,
        token: AccountId,
        amount_set_for_drop: Balance,
        start: Timestamp,
        default_collectable_at_tge: Option<u8>,
        default_cliff: Option<Timestamp>,
        default_vesting: Option<Timestamp>,
    }

    #[derive(scale::Decode, scale::Encode, Debug, Clone, PartialEq)]
    #[cfg_attr(
        feature = "std",
        derive(scale_info::TypeInfo, ink::storage::traits::StorageLayout)
    )]
    pub struct Recipient {
        total_amount: Balance,
        collected: Balance,
        // % of total_amount
        collectable_at_tge: Option<u8>,
        // ms from start user has to wait before either starting vesting, or collecting remaining available.
        cliff: Option<Timestamp>,
        // ms to collect all remaining after collection at tge
        vesting: Option<Timestamp>,
    }

    // === CONTRACT ===
    #[ink(storage)]
    pub struct AzAirdrop {
        admin: AccountId,
        sub_admins_mapping: Mapping<AccountId, AccountId>,
        sub_admins_as_vec: Lazy<Vec<AccountId>>,
        token: AccountId,
        amount_set_for_drop: Balance,
        start: Timestamp,
        recipients: Mapping<AccountId, Recipient>,
        default_collectable_at_tge: Option<u8>,
        default_cliff: Option<Timestamp>,
        default_vesting: Option<Timestamp>,
    }
    impl AzAirdrop {
        #[ink(constructor)]
        pub fn new(
            token: AccountId,
            start: Timestamp,
            default_collectable_at_tge: Option<u8>,
            default_cliff: Option<Timestamp>,
            default_vesting: Option<Timestamp>,
        ) -> Self {
            Self {
                admin: Self::env().caller(),
                sub_admins_mapping: Mapping::default(),
                sub_admins_as_vec: Default::default(),
                token,
                amount_set_for_drop: 0,
                start,
                recipients: Mapping::default(),
                default_collectable_at_tge,
                default_cliff,
                default_vesting,
            }
        }

        // === QUERIES ===
        #[ink(message)]
        pub fn config(&self) -> Config {
            Config {
                admin: self.admin,
                sub_admins: self.sub_admins_as_vec.get_or_default(),
                token: self.token,
                amount_set_for_drop: self.amount_set_for_drop,
                start: self.start,
                default_collectable_at_tge: self.default_collectable_at_tge,
                default_cliff: self.default_cliff,
                default_vesting: self.default_vesting,
            }
        }

        #[ink(message)]
        pub fn show(&self, address: AccountId) -> Result<Recipient> {
            self.recipients
                .get(address)
                .ok_or(AzAirdropError::NotFound("Recipient".to_string()))
        }

        // === HANDLES ===
        // Not a must, but good to have function
        #[ink(message)]
        pub fn acquire_token(&self, amount: Balance, from: AccountId) -> Result<()> {
            let caller: AccountId = Self::env().caller();
            Self::authorise(caller, self.admin)?;
            self.airdrop_has_not_started()?;

            PSP22Ref::transfer_from_builder(
                &self.token,
                from,
                self.env().account_id(),
                amount,
                vec![],
            )
            .call_flags(CallFlags::default())
            .invoke()?;

            Ok(())
        }

        // This is for the sales smart contract to call
        pub fn add_to_recipient(&mut self, address: AccountId, amount: Balance) -> Result<()> {
            self.authorise_to_update_recipient()?;
            self.airdrop_has_not_started()?;
            // Check that balance has enough to cover
            let smart_contract_balance: Balance =
                PSP22Ref::balance_of(&self.token, Self::env().account_id());
            if amount + self.amount_set_for_drop > smart_contract_balance {
                return Err(AzAirdropError::UnprocessableEntity(
                    "Insufficient balance".to_string(),
                ));
            }

            if let Some(mut recipient) = self.recipients.get(address) {
                recipient.total_amount += amount;
                self.recipients.insert(address, &recipient);
            } else {
                let recipient = Recipient {
                    total_amount: amount,
                    collected: 0,
                    collectable_at_tge: self.default_collectable_at_tge,
                    cliff: self.default_cliff,
                    vesting: self.default_vesting,
                };
                self.recipients.insert(address, &recipient);
            }
            self.amount_set_for_drop += amount;

            Ok(())
        }

        #[ink(message)]
        pub fn sub_admins_add(&mut self, address: AccountId) -> Result<Vec<AccountId>> {
            let caller: AccountId = Self::env().caller();
            Self::authorise(caller, self.admin)?;

            let mut sub_admins: Vec<AccountId> = self.sub_admins_as_vec.get_or_default();
            if self.sub_admins_mapping.get(address).is_some() {
                return Err(AzAirdropError::UnprocessableEntity(
                    "Already a sub admin".to_string(),
                ));
            } else {
                sub_admins.push(address.clone());
                self.sub_admins_mapping.insert(address, &address.clone());
            }
            self.sub_admins_as_vec.set(&sub_admins);

            Ok(sub_admins)
        }

        #[ink(message)]
        pub fn sub_admins_remove(&mut self, address: AccountId) -> Result<Vec<AccountId>> {
            let caller: AccountId = Self::env().caller();
            Self::authorise(caller, self.admin)?;

            let mut sub_admins: Vec<AccountId> = self.sub_admins_as_vec.get_or_default();
            if self.sub_admins_mapping.get(address).is_none() {
                return Err(AzAirdropError::UnprocessableEntity(
                    "Not a sub admin".to_string(),
                ));
            } else {
                let index = sub_admins.iter().position(|x| *x == address).unwrap();
                sub_admins.remove(index);
                self.sub_admins_mapping.remove(address);
            }
            self.sub_admins_as_vec.set(&sub_admins);

            Ok(sub_admins)
        }

        // === PRIVATE ===
        fn airdrop_has_not_started(&self) -> Result<()> {
            let block_timestamp: Timestamp = Self::env().block_timestamp();
            if block_timestamp >= self.start {
                return Err(AzAirdropError::UnprocessableEntity(
                    "Airdrop has started".to_string(),
                ));
            }

            Ok(())
        }

        fn authorise(allowed: AccountId, received: AccountId) -> Result<()> {
            if allowed != received {
                return Err(AzAirdropError::Unauthorised);
            }

            Ok(())
        }

        fn authorise_to_update_recipient(&self) -> Result<()> {
            let caller: AccountId = Self::env().caller();
            if caller == self.admin || self.sub_admins_mapping.get(caller).is_some() {
                Ok(())
            } else {
                return Err(AzAirdropError::Unauthorised);
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use ink::env::{
            test::{default_accounts, set_caller, DefaultAccounts},
            DefaultEnvironment,
        };

        const MOCK_START: Timestamp = 654_654;

        // === HELPERS ===
        fn init() -> (DefaultAccounts<DefaultEnvironment>, AzAirdrop) {
            let accounts = default_accounts();
            set_caller::<DefaultEnvironment>(accounts.bob);
            let az_airdrop = AzAirdrop::new(mock_token(), MOCK_START, None, None, None);
            (accounts, az_airdrop)
        }

        fn mock_token() -> AccountId {
            let accounts: DefaultAccounts<DefaultEnvironment> = default_accounts();
            accounts.django
        }

        // === TESTS ===
        // === TEST QUERIES ===
        #[ink::test]
        fn test_config() {
            let (accounts, az_airdrop) = init();
            let config = az_airdrop.config();
            // * it returns the config
            assert_eq!(config.token, mock_token());
            assert_eq!(config.admin, accounts.bob);
            assert_eq!(
                config.sub_admins,
                az_airdrop.sub_admins_as_vec.get_or_default()
            );
            assert_eq!(config.start, MOCK_START);
            assert_eq!(config.default_collectable_at_tge, None);
            assert_eq!(config.default_cliff, None);
            assert_eq!(config.default_vesting, None);
        }

        // === TEST HANDLES ===
        #[ink::test]
        fn test_add_to_recipient() {
            let (accounts, mut az_airdrop) = init();
            let amount: Balance = 5;

            // when caller is not authorised
            set_caller::<DefaultEnvironment>(accounts.charlie);
            // * it raises an error
            let mut result = az_airdrop.add_to_recipient(accounts.charlie, amount);
            assert_eq!(result, Err(AzAirdropError::Unauthorised));
            // when caller is authorised
            set_caller::<DefaultEnvironment>(accounts.bob);
            az_airdrop.sub_admins_add(accounts.charlie).unwrap();
            set_caller::<DefaultEnvironment>(accounts.charlie);
            // = when airdrop has started
            ink::env::test::set_block_timestamp::<ink::env::DefaultEnvironment>(az_airdrop.start);
            // = * it raises an error
            result = az_airdrop.add_to_recipient(accounts.charlie, amount);
            assert_eq!(
                result,
                Err(AzAirdropError::UnprocessableEntity(
                    "Airdrop has started".to_string(),
                ))
            );
            // THE REST NEEDS TO BE IN INK E2E TESTS, SEE BELOW.
            // = when airdrop has not started
            // == when smart contract does not have the balance to cover amount
        }

        #[ink::test]
        fn test_sub_admins_add() {
            let (accounts, mut az_airdrop) = init();
            let new_sub_admin: AccountId = accounts.django;
            // when called by admin
            // = when address is not a sub admin
            let mut result = az_airdrop.sub_admins_add(new_sub_admin);
            result.unwrap();
            // = * it adds the address to sub_admins_vec
            assert_eq!(
                az_airdrop.sub_admins_as_vec.get_or_default(),
                vec![accounts.django]
            );
            // = * it adds the address to sub_admins_mapping
            assert_eq!(
                az_airdrop.sub_admins_mapping.get(new_sub_admin).is_some(),
                true
            );
            // = when already a sub admin
            result = az_airdrop.sub_admins_add(new_sub_admin);
            assert_eq!(
                result,
                Err(AzAirdropError::UnprocessableEntity(
                    "Already a sub admin".to_string()
                ))
            );
            // = * it raises an error
            // when called by non admin
            // * it raises an error
            set_caller::<DefaultEnvironment>(accounts.charlie);
            result = az_airdrop.sub_admins_add(new_sub_admin);
            assert_eq!(result, Err(AzAirdropError::Unauthorised));
        }

        #[ink::test]
        fn test_sub_admins_remove() {
            let (accounts, mut az_airdrop) = init();
            let sub_admin_to_remove: AccountId = accounts.django;
            // when called by admin
            // = when address is not a sub admin
            let mut result = az_airdrop.sub_admins_remove(sub_admin_to_remove);
            assert_eq!(
                result,
                Err(AzAirdropError::UnprocessableEntity(
                    "Not a sub admin".to_string()
                ))
            );
            // = when address is a sub admin
            az_airdrop.sub_admins_add(sub_admin_to_remove).unwrap();
            result = az_airdrop.sub_admins_remove(sub_admin_to_remove);
            result.unwrap();
            // = * it removes the address from sub_admins_vec
            assert_eq!(az_airdrop.sub_admins_as_vec.get_or_default().len(), 0);
            // = * it remove the address from sub_admins_mapping
            assert_eq!(
                az_airdrop
                    .sub_admins_mapping
                    .get(sub_admin_to_remove)
                    .is_some(),
                false
            );
            // = * it raises an error
            // when called by non admin
            // * it raises an error
            set_caller::<DefaultEnvironment>(accounts.charlie);
            result = az_airdrop.sub_admins_remove(sub_admin_to_remove);
            assert_eq!(result, Err(AzAirdropError::Unauthorised));
        }
    }
}
