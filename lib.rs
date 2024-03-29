#![cfg_attr(not(feature = "std"), no_std, no_main)]

pub use self::az_airdrop::AzAirdropRef;

mod errors;

#[ink::contract]
mod az_airdrop {
    use crate::errors::AzAirdropError;
    use ink::{
        codegen::EmitEvent,
        env::CallFlags,
        prelude::string::{String, ToString},
        prelude::{vec, vec::Vec},
        reflect::ContractEventBase,
        storage::{Lazy, Mapping},
    };
    use openbrush::contracts::psp22::PSP22Ref;
    use primitive_types::U256;

    // === TYPES ===
    type Event = <AzAirdrop as ContractEventBase>::Type;
    type Result<T> = core::result::Result<T, AzAirdropError>;

    // === EVENTS ===
    #[ink(event)]
    pub struct RecipientAdd {
        #[ink(topic)]
        address: AccountId,
        amount: Balance,
        caller: AccountId,
        description: Option<String>,
    }

    #[ink(event)]
    pub struct RecipientSubtract {
        #[ink(topic)]
        address: AccountId,
        amount: Balance,
        caller: AccountId,
        description: Option<String>,
    }

    // === STRUCTS ===
    #[derive(Debug, Clone, scale::Encode, scale::Decode)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub struct Config {
        pub admin: AccountId,
        pub sub_admins: Vec<AccountId>,
        pub token: AccountId,
        pub to_be_collected: Balance,
        pub start: Timestamp,
        pub default_collectable_at_tge_percentage: u8,
        pub default_cliff_duration: Timestamp,
        pub default_vesting_duration: Timestamp,
    }

    #[derive(scale::Decode, scale::Encode, Debug, Clone, PartialEq)]
    #[cfg_attr(
        feature = "std",
        derive(scale_info::TypeInfo, ink::storage::traits::StorageLayout)
    )]
    pub struct Recipient {
        pub total_amount: Balance,
        pub collected: Balance,
        // % of total_amount
        pub collectable_at_tge_percentage: u8,
        // ms from start user has to wait before either starting vesting, or collecting remaining available.
        pub cliff_duration: Timestamp,
        // ms to collect all remaining after collection at tge
        pub vesting_duration: Timestamp,
    }

    // === CONTRACT ===
    #[ink(storage)]
    pub struct AzAirdrop {
        admin: AccountId,
        sub_admins_mapping: Mapping<AccountId, AccountId>,
        sub_admins_as_vec: Lazy<Vec<AccountId>>,
        token: AccountId,
        to_be_collected: Balance,
        start: Timestamp,
        recipients: Mapping<AccountId, Recipient>,
        default_collectable_at_tge_percentage: u8,
        default_cliff_duration: Timestamp,
        default_vesting_duration: Timestamp,
    }
    impl AzAirdrop {
        #[ink(constructor)]
        pub fn new(
            token: AccountId,
            start: Timestamp,
            default_collectable_at_tge_percentage: u8,
            default_cliff_duration: Timestamp,
            default_vesting_duration: Timestamp,
        ) -> Result<Self> {
            Self::validate_airdrop_calculation_variables(
                start,
                default_collectable_at_tge_percentage,
                default_cliff_duration,
                default_vesting_duration,
            )?;

            Ok(Self {
                admin: Self::env().caller(),
                sub_admins_mapping: Mapping::default(),
                sub_admins_as_vec: Default::default(),
                token,
                to_be_collected: 0,
                start,
                recipients: Mapping::default(),
                default_collectable_at_tge_percentage,
                default_cliff_duration,
                default_vesting_duration,
            })
        }

        // === QUERIES ===
        // 0 = start (collectable_at_tge)
        // 1 = vesting_start = start + cliff_duration
        // 2 = vesting_end = vesting_start + vesting_duration
        #[ink(message)]
        pub fn collectable_amount(
            &self,
            address: AccountId,
            timestamp: Timestamp,
        ) -> Result<Balance> {
            let recipient: Recipient = self.show(address)?;
            let mut total_collectable_at_time: Balance = 0;
            if timestamp >= self.start {
                // collectable at tge
                let collectable_at_tge: Balance =
                    (U256::from(recipient.collectable_at_tge_percentage)
                        * U256::from(recipient.total_amount)
                        / U256::from(100))
                    .as_u128();
                total_collectable_at_time = collectable_at_tge;
                if recipient.vesting_duration > 0 {
                    // This can't overflow as checks are done in validate_airdrop_calculation_variables
                    let vesting_start: Timestamp = self.start + recipient.cliff_duration;
                    let mut vesting_collectable: Balance = 0;
                    if timestamp >= vesting_start {
                        // This can't overflow
                        let vesting_time_reached: Timestamp = timestamp - vesting_start;
                        // This can't overflow
                        let collectable_during_vesting: Balance =
                            recipient.total_amount - collectable_at_tge;
                        vesting_collectable = (U256::from(vesting_time_reached)
                            * U256::from(collectable_during_vesting)
                            / U256::from(recipient.vesting_duration))
                        .as_u128();
                    }
                    // This can't overflow
                    total_collectable_at_time = total_collectable_at_time + vesting_collectable;
                }
                if total_collectable_at_time > recipient.total_amount {
                    total_collectable_at_time = recipient.total_amount
                }
            }

            Ok(total_collectable_at_time.saturating_sub(recipient.collected))
        }

        #[ink(message)]
        pub fn config(&self) -> Config {
            Config {
                admin: self.admin,
                sub_admins: self.sub_admins_as_vec.get_or_default(),
                token: self.token,
                to_be_collected: self.to_be_collected,
                start: self.start,
                default_collectable_at_tge_percentage: self.default_collectable_at_tge_percentage,
                default_cliff_duration: self.default_cliff_duration,
                default_vesting_duration: self.default_vesting_duration,
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
        pub fn acquire_token(&mut self, amount: Balance, from: AccountId) -> Result<()> {
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

        #[ink(message)]
        pub fn collect(&mut self) -> Result<Balance> {
            let caller: AccountId = Self::env().caller();
            let mut recipient = self.show(caller)?;

            let block_timestamp: Timestamp = Self::env().block_timestamp();
            let collectable_amount: Balance = self.collectable_amount(caller, block_timestamp)?;
            if collectable_amount == 0 {
                return Err(AzAirdropError::UnprocessableEntity(
                    "Amount is zero".to_string(),
                ));
            }

            // transfer to caller
            PSP22Ref::transfer_builder(&self.token, caller, collectable_amount, vec![])
                .call_flags(CallFlags::default())
                .invoke()?;
            // increase recipient's collected
            // These can't overflow, but might as well
            recipient.collected = recipient.collected.saturating_add(collectable_amount);
            self.recipients.insert(caller, &recipient);
            self.to_be_collected = self.to_be_collected.saturating_sub(collectable_amount);

            Ok(collectable_amount)
        }

        // This is for the sales smart contract to call
        #[ink(message)]
        pub fn recipient_add(
            &mut self,
            address: AccountId,
            amount: Balance,
            description: Option<String>,
        ) -> Result<Recipient> {
            self.authorise_to_update_recipient()?;
            self.airdrop_has_not_started()?;
            if let Some(new_to_be_collected) = amount.checked_add(self.to_be_collected) {
                // Check that balance has enough to cover
                let smart_contract_balance: Balance =
                    PSP22Ref::balance_of(&self.token, Self::env().account_id());
                if new_to_be_collected > smart_contract_balance {
                    return Err(AzAirdropError::UnprocessableEntity(
                        "Insufficient balance".to_string(),
                    ));
                }

                let mut recipient: Recipient = self.recipients.get(address).unwrap_or(Recipient {
                    total_amount: 0,
                    collected: 0,
                    collectable_at_tge_percentage: self.default_collectable_at_tge_percentage,
                    cliff_duration: self.default_cliff_duration,
                    vesting_duration: self.default_vesting_duration,
                });
                // This can't overflow
                recipient.total_amount += amount;
                self.recipients.insert(address, &recipient);
                self.to_be_collected = new_to_be_collected;

                // emit event
                Self::emit_event(
                    self.env(),
                    Event::RecipientAdd(RecipientAdd {
                        address,
                        amount,
                        caller: Self::env().caller(),
                        description,
                    }),
                );

                Ok(recipient)
            } else {
                return Err(AzAirdropError::UnprocessableEntity(
                    "Amount will cause to_be_collected to overflow".to_string(),
                ));
            }
        }

        #[ink(message)]
        pub fn recipient_subtract(
            &mut self,
            address: AccountId,
            amount: Balance,
            description: Option<String>,
        ) -> Result<Recipient> {
            self.authorise_to_update_recipient()?;
            self.airdrop_has_not_started()?;
            let mut recipient = self.show(address)?;
            if amount > recipient.total_amount {
                return Err(AzAirdropError::UnprocessableEntity(
                    "Amount is greater than recipient's total amount".to_string(),
                ));
            }

            // Update recipient
            // This can't overflow because of the above check
            recipient.total_amount -= amount;
            self.recipients.insert(address, &recipient);

            // Update config
            // This can't overflow but might as well
            self.to_be_collected = self.to_be_collected.saturating_sub(amount);

            // emit event
            Self::emit_event(
                self.env(),
                Event::RecipientSubtract(RecipientSubtract {
                    address,
                    amount,
                    caller: Self::env().caller(),
                    description,
                }),
            );

            Ok(recipient)
        }

        #[ink(message)]
        pub fn return_spare_tokens(&mut self) -> Result<Balance> {
            let caller: AccountId = Self::env().caller();
            let contract_address: AccountId = Self::env().account_id();
            Self::authorise(caller, self.admin)?;

            let balance: Balance = PSP22Ref::balance_of(&self.token, contract_address);
            // These can't overflow, but might as well
            let spare_amount: Balance = balance.saturating_sub(self.to_be_collected);
            if spare_amount > 0 {
                PSP22Ref::transfer_builder(&self.token, caller, spare_amount, vec![])
                    .call_flags(CallFlags::default())
                    .invoke()?;
            } else {
                return Err(AzAirdropError::UnprocessableEntity(
                    "Amount is zero".to_string(),
                ));
            }

            Ok(spare_amount)
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

        // #[derive(Debug, Clone, scale::Encode, scale::Decode)]
        // #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
        // pub struct Config {
        //     admin: AccountId,
        //     sub_admins: Vec<AccountId>,
        //     token: AccountId,
        //     to_be_collected: Balance,
        //     start: Timestamp,
        //     default_collectable_at_tge_percentage: u8,
        //     default_cliff_duration: Timestamp,
        //     default_vesting_duration: Timestamp,
        // }
        #[ink(message)]
        pub fn update_config(
            &mut self,
            admin: Option<AccountId>,
            start: Option<Timestamp>,
            default_collectable_at_tge_percentage: Option<u8>,
            default_cliff_duration: Option<Timestamp>,
            default_vesting_duration: Option<Timestamp>,
        ) -> Result<()> {
            let caller: AccountId = Self::env().caller();
            Self::authorise(caller, self.admin)?;

            if let Some(admin_unwrapped) = admin {
                self.admin = admin_unwrapped
            }
            if let Some(start_unwrapped) = start {
                let block_timestamp: Timestamp = Self::env().block_timestamp();
                if start_unwrapped > block_timestamp {
                    if self.to_be_collected == 0 {
                        self.start = start_unwrapped
                    } else {
                        return Err(AzAirdropError::UnprocessableEntity(
                            "to_be_collected must be zero when changing start time".to_string(),
                        ));
                    }
                } else {
                    return Err(AzAirdropError::UnprocessableEntity(
                        "New start time must be in the future".to_string(),
                    ));
                }
            }
            if let Some(default_collectable_at_tge_percentage_unwrapped) =
                default_collectable_at_tge_percentage
            {
                self.default_collectable_at_tge_percentage =
                    default_collectable_at_tge_percentage_unwrapped
            }
            if let Some(default_cliff_duration_unwrapped) = default_cliff_duration {
                self.default_cliff_duration = default_cliff_duration_unwrapped
            }
            if let Some(default_vesting_duration_unwrapped) = default_vesting_duration {
                self.default_vesting_duration = default_vesting_duration_unwrapped
            }
            Self::validate_airdrop_calculation_variables(
                self.start,
                self.default_collectable_at_tge_percentage,
                self.default_cliff_duration,
                self.default_vesting_duration,
            )?;

            // Will not let me check exact error
            // when Config is returned
            Ok(())
        }

        #[ink(message)]
        pub fn update_recipient(
            &mut self,
            address: AccountId,
            collectable_at_tge_percentage: Option<u8>,
            cliff_duration: Option<Timestamp>,
            vesting_duration: Option<Timestamp>,
        ) -> Result<Recipient> {
            self.authorise_to_update_recipient()?;
            self.airdrop_has_not_started()?;
            let mut recipient: Recipient = self.show(address)?;

            if let Some(collectable_at_tge_percentage_unwrapped) = collectable_at_tge_percentage {
                recipient.collectable_at_tge_percentage = collectable_at_tge_percentage_unwrapped
            }
            if let Some(cliff_duration_unwrapped) = cliff_duration {
                recipient.cliff_duration = cliff_duration_unwrapped
            }
            if let Some(vesting_duration_unwrapped) = vesting_duration {
                recipient.vesting_duration = vesting_duration_unwrapped
            }
            Self::validate_airdrop_calculation_variables(
                self.start,
                recipient.collectable_at_tge_percentage,
                recipient.cliff_duration,
                recipient.vesting_duration,
            )?;

            self.recipients.insert(address, &recipient);

            Ok(recipient)
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

        fn emit_event<EE: EmitEvent<Self>>(emitter: EE, event: Event) {
            emitter.emit_event(event);
        }

        fn validate_airdrop_calculation_variables(
            start: Timestamp,
            collectable_at_tge_percentage: u8,
            cliff_duration: Timestamp,
            vesting_duration: Timestamp,
        ) -> Result<()> {
            if collectable_at_tge_percentage > 100 {
                return Err(AzAirdropError::UnprocessableEntity(
                    "collectable_at_tge_percentage must be less than or equal to 100".to_string(),
                ));
            } else if collectable_at_tge_percentage == 100 {
                if cliff_duration > 0 || vesting_duration > 0 {
                    return Err(AzAirdropError::UnprocessableEntity(
                        "cliff_duration and vesting_duration must be 0 when collectable_tge_percentage is 100"
                            .to_string(),
                    ));
                }
            } else if vesting_duration == 0 {
                return Err(AzAirdropError::UnprocessableEntity(
                    "vesting_duration must be greater than 0 when collectable_tge_percentage is not 100"
                        .to_string(),
                ));
            }
            // This can't over flow because all values are u64
            let end_timestamp: u128 =
                u128::from(start) + u128::from(cliff_duration) + u128::from(vesting_duration);
            if end_timestamp > Timestamp::MAX.into() {
                return Err(AzAirdropError::UnprocessableEntity(
                    "Combination of start, cliff_duration and vesting_duration exceeds limit"
                        .to_string(),
                ));
            }

            Ok(())
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
            let az_airdrop = AzAirdrop::new(mock_token(), MOCK_START, 100, 0, 0).unwrap();
            (accounts, az_airdrop)
        }

        fn mock_token() -> AccountId {
            let accounts: DefaultAccounts<DefaultEnvironment> = default_accounts();
            accounts.django
        }

        // === TESTS ===
        // === TEST CONSTRUCTOR ===
        #[ink::test]
        fn test_new() {
            let accounts: DefaultAccounts<DefaultEnvironment> = default_accounts();
            set_caller::<DefaultEnvironment>(accounts.bob);
            let result = AzAirdrop::new(mock_token(), MOCK_START, 0, 0, 0);
            assert!(result.is_err());
        }

        // === TEST QUERIES ===
        #[ink::test]
        fn test_collectable_amount() {
            let (accounts, mut az_airdrop) = init();
            let recipient_address: AccountId = accounts.django;
            let mut recipient: Recipient = Recipient {
                total_amount: 100,
                collected: 0,
                collectable_at_tge_percentage: 100,
                cliff_duration: 0,
                vesting_duration: 0,
            };
            // when recipient does not exist
            // * it returns an error
            let mut result = az_airdrop.collectable_amount(recipient_address, 0);
            assert_eq!(
                result,
                Err(AzAirdropError::NotFound("Recipient".to_string(),))
            );
            // when recipient exists
            az_airdrop.recipients.insert(recipient_address, &recipient);
            // = when provided timestamp is before the start time
            // = * it returns zero
            result = az_airdrop.collectable_amount(recipient_address, MOCK_START - 1);
            let mut result_unwrapped: Balance = result.unwrap();
            assert_eq!(result_unwrapped, 0);
            // = when provided timestamp is greater than or equal to start time
            // == when collectable_at_tge_percentage is positive
            // === when collectable_at_tge_percentagne is 100
            // === * it returns the total_amount
            result = az_airdrop.collectable_amount(recipient_address, MOCK_START);
            result_unwrapped = result.unwrap();
            assert_eq!(result_unwrapped, recipient.total_amount);
            // === when collectable_at_tge_percentage is 20
            // ==== when vesting time has not been reached
            // ==== * it returns 20
            recipient = az_airdrop
                .update_recipient(recipient_address, Some(20), Some(1), Some(100))
                .unwrap();
            result = az_airdrop.collectable_amount(recipient_address, MOCK_START);
            result_unwrapped = result.unwrap();
            assert_eq!(result_unwrapped, 20);
            result = az_airdrop.collectable_amount(recipient_address, MOCK_START + 1);
            result_unwrapped = result.unwrap();
            assert_eq!(result_unwrapped, 20);
            // ==== when partial vesting time has been reached
            result = az_airdrop
                .collectable_amount(recipient_address, MOCK_START + recipient.cliff_duration + 2);
            // ==== * it returns the partial amount
            result_unwrapped = result.unwrap();
            assert_eq!(result_unwrapped, 20 + (2 * 80 / 100));
            // ==== when total vesting time has been reached
            result = az_airdrop.collectable_amount(
                recipient_address,
                MOCK_START + recipient.cliff_duration + recipient.vesting_duration * 1_000_000,
            );
            // ==== * it returns the total amount
            result_unwrapped = result.unwrap();
            assert_eq!(result_unwrapped, recipient.total_amount);
            // ==== * it factors in recipient.collected
            recipient.collected = 20;
            az_airdrop.recipients.insert(recipient_address, &recipient);
            result = az_airdrop.collectable_amount(
                recipient_address,
                MOCK_START + recipient.cliff_duration + recipient.vesting_duration,
            );
            result_unwrapped = result.unwrap();
            assert_eq!(result_unwrapped, recipient.total_amount - 20);
        }

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
            assert_eq!(config.default_collectable_at_tge_percentage, 100);
            assert_eq!(config.default_cliff_duration, 0);
            assert_eq!(config.default_vesting_duration, 0);
        }

        // === TEST HANDLES ===
        #[ink::test]
        fn test_recipient_add() {
            let (accounts, mut az_airdrop) = init();
            let amount: Balance = 5;

            // when caller is not authorised
            set_caller::<DefaultEnvironment>(accounts.charlie);
            // * it raises an error
            let mut result = az_airdrop.recipient_add(accounts.charlie, amount, None);
            assert_eq!(result, Err(AzAirdropError::Unauthorised));
            // when caller is authorised
            set_caller::<DefaultEnvironment>(accounts.bob);
            az_airdrop.sub_admins_add(accounts.charlie).unwrap();
            set_caller::<DefaultEnvironment>(accounts.charlie);
            // = when airdrop has started
            ink::env::test::set_block_timestamp::<ink::env::DefaultEnvironment>(az_airdrop.start);
            // = * it raises an error
            result = az_airdrop.recipient_add(accounts.charlie, amount, None);
            assert_eq!(
                result,
                Err(AzAirdropError::UnprocessableEntity(
                    "Airdrop has started".to_string(),
                ))
            );
            // = when airdrop has not started
            ink::env::test::set_block_timestamp::<ink::env::DefaultEnvironment>(
                az_airdrop.start - 1,
            );
            // == when amount will cause overflow
            az_airdrop.to_be_collected = Balance::MAX;
            // == * it raises an error
            result = az_airdrop.recipient_add(accounts.charlie, amount, None);
            assert_eq!(
                result,
                Err(AzAirdropError::UnprocessableEntity(
                    "Amount will cause to_be_collected to overflow".to_string(),
                ))
            );
            // == when amount won't cause overflow
            // THE REST NEEDS TO BE IN INK E2E TESTS, SEE BELOW.
        }

        #[ink::test]
        fn test_collect() {
            let (accounts, mut az_airdrop) = init();
            // when recipient with caller's address does not exist
            // * it raises an error
            let mut result = az_airdrop.collect();
            assert_eq!(
                result,
                Err(AzAirdropError::NotFound("Recipient".to_string()))
            );
            // when recipient with caller's address exists
            az_airdrop.recipients.insert(
                accounts.bob,
                &Recipient {
                    total_amount: 5,
                    collected: 0,
                    collectable_at_tge_percentage: 100,
                    cliff_duration: 0,
                    vesting_duration: 0,
                },
            );
            // = when collectable amount is zero
            ink::env::test::set_block_timestamp::<ink::env::DefaultEnvironment>(
                az_airdrop.start - 1,
            );
            // = * it raises an error
            result = az_airdrop.collect();
            assert_eq!(
                result,
                Err(AzAirdropError::UnprocessableEntity(
                    "Amount is zero".to_string(),
                ))
            );
            // = when collectable amount is positive
            // THE REST NEEDS TO HAPPEN IN INTEGRATION TESTS
        }

        #[ink::test]
        fn test_return_spare_token() {
            let (accounts, mut az_airdrop) = init();
            // when called by admin
            // THIS NEEDS TO HAPPEN IN INTEGRATION TESTS
            // when called by non-admin
            // * it raises an error
            set_caller::<DefaultEnvironment>(accounts.charlie);
            let result = az_airdrop.return_spare_tokens();
            assert_eq!(result, Err(AzAirdropError::Unauthorised));
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

        #[ink::test]
        fn test_recipient_subtract() {
            let (accounts, mut az_airdrop) = init();
            let amount: Balance = 5;
            let recipient_address: AccountId = accounts.django;
            // when called by an admin or sub-admin
            // = when airdrop has started
            ink::env::test::set_block_timestamp::<ink::env::DefaultEnvironment>(az_airdrop.start);
            // = * it raises an error
            let mut result = az_airdrop.recipient_subtract(recipient_address, amount, None);
            assert_eq!(
                result,
                Err(AzAirdropError::UnprocessableEntity(
                    "Airdrop has started".to_string(),
                ))
            );
            // = when airdrop has not started
            ink::env::test::set_block_timestamp::<ink::env::DefaultEnvironment>(
                az_airdrop.start - 1,
            );
            // == when recipient does not exist
            // == * it raises an error
            result = az_airdrop.recipient_subtract(recipient_address, amount, None);
            assert_eq!(
                result,
                Err(AzAirdropError::NotFound("Recipient".to_string()))
            );
            // == when recipient exists
            az_airdrop.recipients.insert(
                recipient_address,
                &Recipient {
                    total_amount: amount,
                    collected: 0,
                    collectable_at_tge_percentage: 0,
                    cliff_duration: 0,
                    vesting_duration: 0,
                },
            );
            // === when amount is greater than the recipient's total amount
            // === * it returns an error
            result = az_airdrop.recipient_subtract(recipient_address, amount + 1, None);
            assert_eq!(
                result,
                Err(AzAirdropError::UnprocessableEntity(
                    "Amount is greater than recipient's total amount".to_string()
                ))
            );
            // === when amount is less than or equal to the recipient's total amount
            az_airdrop.to_be_collected += amount;
            // === * it reduces the total_amount by the amount
            az_airdrop
                .recipient_subtract(recipient_address, amount - 1, None)
                .unwrap();
            let recipient: Recipient = az_airdrop.recipients.get(recipient_address).unwrap();
            assert_eq!(recipient.total_amount, 1);
            // when called by non-admin or non-sub-admin
            set_caller::<DefaultEnvironment>(accounts.charlie);
            // * it raises an error
            result = az_airdrop.recipient_subtract(recipient_address, amount, None);
            assert_eq!(result, Err(AzAirdropError::Unauthorised));
            // === * it reduces the total_amount
            assert_eq!(az_airdrop.to_be_collected, 1);
        }

        #[ink::test]
        fn test_update_config() {
            let (accounts, mut az_airdrop) = init();
            // when called by admin
            // = when new admin is provided
            az_airdrop
                .update_config(Some(accounts.django), None, None, None, None)
                .unwrap();
            // = * it updates the admin
            let config: Config = az_airdrop.config();
            assert_eq!(config.admin, accounts.django);
            set_caller::<DefaultEnvironment>(accounts.django);
            // = when new start is provided
            // == when new start is before or equal to current time stamp
            let current_timestamp: Timestamp = 5;
            ink::env::test::set_block_timestamp::<ink::env::DefaultEnvironment>(current_timestamp);
            let result = az_airdrop.update_config(None, Some(current_timestamp), None, None, None);
            // == * it raises an error
            assert_eq!(
                result,
                Err(AzAirdropError::UnprocessableEntity(
                    "New start time must be in the future".to_string()
                ))
            );
            // == when new start is after current time stamp
            // === when to_be_collected is positive
            az_airdrop.to_be_collected = 1;
            // === * it raises an error
            let result =
                az_airdrop.update_config(None, Some(current_timestamp + 1), None, None, None);
            assert_eq!(
                result,
                Err(AzAirdropError::UnprocessableEntity(
                    "to_be_collected must be zero when changing start time".to_string()
                ))
            );
            // === when to_be_collected is zero
            az_airdrop.to_be_collected = 0;
            // === * it updates the start time
            az_airdrop
                .update_config(None, Some(current_timestamp + 1), None, None, None)
                .unwrap();
            let mut config: Config = az_airdrop.config();
            assert_eq!(config.start, current_timestamp + 1);
            // = when new default_collectable_at_tge_percentage is provided
            // == when airdrop calculation variable combination is invalid
            // == * it raises an error
            let result = az_airdrop.update_config(None, None, Some(50), None, None);
            assert_eq!(
                result,
                Err(AzAirdropError::UnprocessableEntity(
                    "vesting_duration must be greater than 0 when collectable_tge_percentage is not 100"
                        .to_string(),
                ))
            );
            // == when combination of start, cliff_duration and vesting_duration exceeds Timestamp max
            let result = az_airdrop.update_config(
                None,
                None,
                Some(50),
                Some((Timestamp::MAX / 2) - az_airdrop.start + 2),
                Some(Timestamp::MAX / 2),
            );
            assert_eq!(
                result,
                Err(AzAirdropError::UnprocessableEntity(
                    "Combination of start, cliff_duration and vesting_duration exceeds limit"
                        .to_string(),
                ))
            );
            // == when airdrop calculation variable combination is valid
            az_airdrop
                .update_config(None, None, Some(50), Some(50), Some(50))
                .unwrap();
            // == * it updates the default_collectable_at_tge_percentage
            config = az_airdrop.config();
            assert_eq!(config.default_collectable_at_tge_percentage, 50);
            assert_eq!(config.default_cliff_duration, 50);
            assert_eq!(config.default_vesting_duration, 50);
            // No need to test the other default fields as test above does that
            // when called by non-admin
            set_caller::<DefaultEnvironment>(accounts.charlie);
            // * it raises an error
            let result = az_airdrop.update_config(None, None, None, None, None);
            assert_eq!(result, Err(AzAirdropError::Unauthorised));
        }

        #[ink::test]
        fn test_update_recipient() {
            let (accounts, mut az_airdrop) = init();
            let recipient: AccountId = accounts.django;
            // when called by an admin or sub-admin
            // = when airdrop has started
            ink::env::test::set_block_timestamp::<ink::env::DefaultEnvironment>(az_airdrop.start);
            // = * it raises an error
            let mut result = az_airdrop.update_recipient(recipient, None, None, None);
            assert_eq!(
                result,
                Err(AzAirdropError::UnprocessableEntity(
                    "Airdrop has started".to_string(),
                ))
            );
            // = when airdrop has not started
            ink::env::test::set_block_timestamp::<ink::env::DefaultEnvironment>(
                az_airdrop.start - 1,
            );
            // == when recipient does not exist
            // == * it raises an error
            result = az_airdrop.update_recipient(recipient, None, None, None);
            assert_eq!(
                result,
                Err(AzAirdropError::NotFound("Recipient".to_string(),))
            );
            // == when recipient exists
            az_airdrop.recipients.insert(
                recipient,
                &Recipient {
                    total_amount: 5,
                    collected: 0,
                    collectable_at_tge_percentage: 0,
                    cliff_duration: 0,
                    vesting_duration: 0,
                },
            );
            // == * it updates the provided fields
            az_airdrop
                .update_recipient(recipient, Some(5), Some(5), Some(5))
                .unwrap();
            let updated_recipient: Recipient = az_airdrop.recipients.get(recipient).unwrap();
            assert_eq!(
                updated_recipient,
                Recipient {
                    total_amount: 5,
                    collected: 0,
                    collectable_at_tge_percentage: 5,
                    cliff_duration: 5,
                    vesting_duration: 5
                }
            );
            // === when recipient's collectable_at_tge_percentage is greater than 100
            // === * it raises an error
            result = az_airdrop.update_recipient(recipient, Some(101), None, None);
            assert_eq!(
                result,
                Err(AzAirdropError::UnprocessableEntity(
                    "collectable_at_tge_percentage must be less than or equal to 100".to_string()
                ))
            );
            // === when recipient's collectable_at_tge_percentage is 100
            // ==== when cliff_duration or vesting_duration is positive
            // ==== * it raises an error
            result = az_airdrop.update_recipient(recipient, Some(100), Some(1), Some(0));
            assert_eq!(
                result,
                Err(AzAirdropError::UnprocessableEntity(
                    "cliff_duration and vesting_duration must be 0 when collectable_tge_percentage is 100".to_string()
                ))
            );
            result = az_airdrop.update_recipient(recipient, Some(100), Some(0), Some(1));
            assert_eq!(
                result,
                Err(AzAirdropError::UnprocessableEntity(
                    "cliff_duration and vesting_duration must be 0 when collectable_tge_percentage is 100".to_string()
                ))
            );
            // === when recipient's collectable_at_tge_percentage is less than 100
            // ==== when vesting_duration is zero
            // ==== * it raises an error
            result = az_airdrop.update_recipient(recipient, Some(0), None, Some(0));
            assert_eq!(
                result,
                Err(AzAirdropError::UnprocessableEntity(
                    "vesting_duration must be greater than 0 when collectable_tge_percentage is not 100".to_string()
                ))
            );

            // when called by non-admin or non-sub-admin
            set_caller::<DefaultEnvironment>(accounts.charlie);
            // * it raises an error
            result = az_airdrop.update_recipient(recipient, None, None, None);
            assert_eq!(result, Err(AzAirdropError::Unauthorised));
        }
    }

    #[cfg(all(test, feature = "e2e-tests"))]
    mod e2e_tests {
        use super::*;
        use crate::az_airdrop::AzAirdropRef;
        use az_button::ButtonRef;
        use ink_e2e::build_message;
        use ink_e2e::Keypair;
        use openbrush::contracts::traits::psp22::psp22_external::PSP22;

        // === CONSTANT ===
        const MOCK_AMOUNT: Balance = 250;
        const MOCK_START: Timestamp = 2708075722737;

        // === TYPES ===
        type E2EResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;

        // === HELPERS ===
        fn account_id(k: Keypair) -> AccountId {
            AccountId::try_from(k.public_key().to_account_id().as_ref())
                .expect("account keyring has a valid account id")
        }

        // === TEST HANDLES ===
        // This is just to test when cheque has a token address associated with it
        #[ink_e2e::test]
        async fn test_recipient_add(mut client: ::ink_e2e::Client<C, E>) -> E2EResult<()> {
            let bob_account_id: AccountId = account_id(ink_e2e::bob());

            // Instantiate token
            let token_constructor = ButtonRef::new(
                MOCK_AMOUNT,
                Some("DIBS".to_string()),
                Some("DIBS".to_string()),
                12,
            );
            let token_id: AccountId = client
                .instantiate("az_button", &ink_e2e::alice(), token_constructor, 0, None)
                .await
                .expect("Token instantiate failed")
                .account_id;

            // Instantiate airdrop smart contract
            let default_collectable_at_tge_percentage: u8 = 20;
            let default_cliff_duration: Timestamp = 0;
            let default_vesting_duration: Timestamp = 31_556_952_000;
            let airdrop_constructor = AzAirdropRef::new(
                token_id,
                MOCK_START,
                default_collectable_at_tge_percentage,
                default_cliff_duration,
                default_vesting_duration,
            );
            let airdrop_id: AccountId = client
                .instantiate(
                    "az_airdrop",
                    &ink_e2e::alice(),
                    airdrop_constructor,
                    0,
                    None,
                )
                .await
                .expect("Airdrop instantiate failed")
                .account_id;

            // when caller is authorised
            // = when airdrop has not started
            // == when smart contract does not have the balance to cover amount
            // == * it raises an error
            let recipient_add_message = build_message::<AzAirdropRef>(airdrop_id)
                .call(|airdrop| airdrop.recipient_add(bob_account_id, 1, None));
            let result = client
                .call_dry_run(&ink_e2e::alice(), &recipient_add_message, 0, None)
                .await
                .return_value();
            assert_eq!(
                result,
                Err(AzAirdropError::UnprocessableEntity(
                    "Insufficient balance".to_string()
                ))
            );
            // == when smart contract has the balance to cover amount
            let transfer_message = build_message::<ButtonRef>(token_id)
                .call(|button| button.transfer(airdrop_id, 1, vec![]));
            let transfer_result = client
                .call(&ink_e2e::alice(), transfer_message, 0, None)
                .await
                .unwrap()
                .dry_run
                .exec_result
                .result;
            assert!(transfer_result.is_ok());
            // == * it adds to the recipient's total_amount and sets details with defaults if not provided and new
            let recipient_add_message = build_message::<AzAirdropRef>(airdrop_id)
                .call(|airdrop| airdrop.recipient_add(bob_account_id, 1, None));
            client
                .call(&ink_e2e::alice(), recipient_add_message, 0, None)
                .await
                .unwrap();
            let show_message = build_message::<AzAirdropRef>(airdrop_id)
                .call(|airdrop| airdrop.show(bob_account_id));
            let recipient = client
                .call_dry_run(&ink_e2e::alice(), &show_message, 0, None)
                .await
                .return_value()
                .unwrap();
            assert_eq!(recipient.total_amount, 1);
            assert_eq!(
                recipient.collectable_at_tge_percentage,
                default_collectable_at_tge_percentage
            );
            assert_eq!(recipient.cliff_duration, default_cliff_duration);
            assert_eq!(recipient.vesting_duration, default_vesting_duration);
            // == * it adds to the to_be_collected
            let config_message =
                build_message::<AzAirdropRef>(airdrop_id).call(|airdrop| airdrop.config());
            let config = client
                .call_dry_run(&ink_e2e::alice(), &config_message, 0, None)
                .await
                .return_value();
            assert_eq!(config.to_be_collected, 1);

            Ok(())
        }

        // I CAN'T MODIFY TIMESTAMP WITH INK_E2E, PLEASE TEST MANUALLY THAT
        // = * it transfers the collectable amount to the recipient
        // = * it increases the recipient's collected by the collectable amount
        // = * it reduces the to_be_collected by the collectable amount
        // #[ink_e2e::test]
        // async fn test_collect(mut client: ::ink_e2e::Client<C, E>) -> E2EResult<()> {}

        #[ink_e2e::test]
        async fn test_return_spare_token(mut client: ::ink_e2e::Client<C, E>) -> E2EResult<()> {
            let alice_account_id: AccountId = account_id(ink_e2e::alice());

            // Instantiate token
            let token_constructor = ButtonRef::new(
                MOCK_AMOUNT,
                Some("DIBS".to_string()),
                Some("DIBS".to_string()),
                12,
            );
            let token_id: AccountId = client
                .instantiate("az_button", &ink_e2e::alice(), token_constructor, 0, None)
                .await
                .expect("Token instantiate failed")
                .account_id;

            // Instantiate airdrop smart contract
            let default_collectable_at_tge_percentage: u8 = 20;
            let default_cliff_duration: Timestamp = 0;
            let default_vesting_duration: Timestamp = 31_556_952_000;
            let airdrop_constructor = AzAirdropRef::new(
                token_id,
                MOCK_START,
                default_collectable_at_tge_percentage,
                default_cliff_duration,
                default_vesting_duration,
            );
            let airdrop_id: AccountId = client
                .instantiate(
                    "az_airdrop",
                    &ink_e2e::alice(),
                    airdrop_constructor,
                    0,
                    None,
                )
                .await
                .expect("Airdrop instantiate failed")
                .account_id;

            // when called by an admin
            // = when there is no spare token
            // = * it raises an error
            let return_spare_tokens_message = build_message::<AzAirdropRef>(airdrop_id)
                .call(|airdrop| airdrop.return_spare_tokens());
            let result = client
                .call_dry_run(&ink_e2e::alice(), &return_spare_tokens_message, 0, None)
                .await
                .return_value();
            assert_eq!(
                result,
                Err(AzAirdropError::UnprocessableEntity(
                    "Amount is zero".to_string()
                ))
            );
            // = when there is spare token
            let transfer_message = build_message::<ButtonRef>(token_id)
                .call(|token| token.transfer(airdrop_id, 1, vec![]));
            let transfer_result = client
                .call(&ink_e2e::alice(), transfer_message, 0, None)
                .await
                .unwrap()
                .dry_run
                .exec_result
                .result;
            assert!(transfer_result.is_ok());
            // = * it returns the spare token to admin
            let return_spare_tokens_message = build_message::<AzAirdropRef>(airdrop_id)
                .call(|airdrop| airdrop.return_spare_tokens());
            client
                .call(&ink_e2e::alice(), return_spare_tokens_message, 0, None)
                .await
                .unwrap();
            let balance_message =
                build_message::<ButtonRef>(token_id).call(|button| button.balance_of(airdrop_id));
            let result = client
                .call_dry_run(&ink_e2e::alice(), &balance_message, 0, None)
                .await
                .return_value();
            assert_eq!(result, 0);
            let balance_message = build_message::<ButtonRef>(token_id)
                .call(|button| button.balance_of(alice_account_id));
            let result = client
                .call_dry_run(&ink_e2e::alice(), &balance_message, 0, None)
                .await
                .return_value();
            assert_eq!(result, MOCK_AMOUNT);

            Ok(())
        }
    }
}
