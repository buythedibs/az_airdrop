#![cfg_attr(not(feature = "std"), no_std, no_main)]

mod errors;

#[ink::contract]
mod az_airdrop {
    use crate::errors::AzAirdropError;
    use ink::{
        env::CallFlags,
        prelude::string::ToString,
        prelude::{vec, vec::Vec},
        reflect::ContractEventBase,
        storage::{Lazy, Mapping},
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
        to_be_collected: Balance,
        start: Timestamp,
        default_collectable_at_tge_percentage: u8,
        default_cliff_duration: Timestamp,
        default_vesting_duration: Timestamp,
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
        collectable_at_tge_percentage: u8,
        // ms from start user has to wait before either starting vesting, or collecting remaining available.
        cliff_duration: Timestamp,
        // ms to collect all remaining after collection at tge
        vesting_duration: Timestamp,
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
        ) -> Self {
            Self {
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
            }
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
                    Balance::from(recipient.collectable_at_tge_percentage) * recipient.total_amount
                        / 100;
                total_collectable_at_time = collectable_at_tge;
                if recipient.vesting_duration > 0 {
                    // cliff_duration
                    let vesting_start: Timestamp = self.start + recipient.cliff_duration;
                    let mut vesting_collectable: Balance = 0;
                    if timestamp >= vesting_start {
                        let vesting_time_reached: Timestamp = timestamp - vesting_start;
                        let collectable_during_vesting: Balance =
                            recipient.total_amount - collectable_at_tge;
                        vesting_collectable = Balance::from(vesting_time_reached)
                            * collectable_during_vesting
                            / Balance::from(recipient.vesting_duration)
                    }
                    total_collectable_at_time = total_collectable_at_time + vesting_collectable;
                }
                if total_collectable_at_time > recipient.total_amount {
                    total_collectable_at_time = recipient.total_amount
                }
            }

            Ok(total_collectable_at_time - recipient.collected)
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
        #[ink(message)]
        pub fn add_to_recipient(
            &mut self,
            address: AccountId,
            amount: Balance,
            collectable_at_tge_percentage: Option<u8>,
            cliff_duration: Option<Timestamp>,
            vesting_duration: Option<Timestamp>,
        ) -> Result<Recipient> {
            self.authorise_to_update_recipient()?;
            self.airdrop_has_not_started()?;
            // Check that balance has enough to cover
            let smart_contract_balance: Balance =
                PSP22Ref::balance_of(&self.token, Self::env().account_id());
            if amount + self.to_be_collected > smart_contract_balance {
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
            recipient.total_amount += amount;
            self.recipients.insert(address, &recipient);
            self.update_recipient(
                address,
                collectable_at_tge_percentage,
                cliff_duration,
                vesting_duration,
            )?;
            self.to_be_collected += amount;

            Ok(recipient)
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
            recipient.collected += collectable_amount;
            self.recipients.insert(caller, &recipient);

            // should we reduce amount committed to airdrop for returning tokens logic?
            self.to_be_collected -= collectable_amount;

            Ok(collectable_amount)
        }

        #[ink(message)]
        pub fn subtract_from_recipient(
            &mut self,
            address: AccountId,
            amount: Balance,
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
            recipient.total_amount -= amount;
            self.recipients.insert(address, &recipient);

            // Update config
            self.to_be_collected -= amount;

            Ok(recipient)
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
            if recipient.collectable_at_tge_percentage > 100 {
                return Err(AzAirdropError::UnprocessableEntity(
                    "collectable_at_tge_percentage must be less than or equal to 100".to_string(),
                ));
            } else if recipient.collectable_at_tge_percentage == 100 {
                if recipient.cliff_duration > 0 || recipient.vesting_duration > 0 {
                    return Err(AzAirdropError::UnprocessableEntity(
                        "cliff_duration and vesting_duration must be 0 when collectable_tge_percentage is 100"
                            .to_string(),
                    ));
                }
            } else if recipient.vesting_duration == 0 {
                return Err(AzAirdropError::UnprocessableEntity(
                    "vesting_duration must be greater than 0 when collectable_tge_percentage is not 100"
                        .to_string(),
                ));
            }

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
            let az_airdrop = AzAirdrop::new(mock_token(), MOCK_START, 0, 0, 0);
            (accounts, az_airdrop)
        }

        fn mock_token() -> AccountId {
            let accounts: DefaultAccounts<DefaultEnvironment> = default_accounts();
            accounts.django
        }

        // === TESTS ===
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
            assert_eq!(config.default_collectable_at_tge_percentage, 0);
            assert_eq!(config.default_cliff_duration, 0);
            assert_eq!(config.default_vesting_duration, 0);
        }

        // === TEST HANDLES ===
        #[ink::test]
        fn test_add_to_recipient() {
            let (accounts, mut az_airdrop) = init();
            let amount: Balance = 5;

            // when caller is not authorised
            set_caller::<DefaultEnvironment>(accounts.charlie);
            // * it raises an error
            let mut result =
                az_airdrop.add_to_recipient(accounts.charlie, amount, None, None, None);
            assert_eq!(result, Err(AzAirdropError::Unauthorised));
            // when caller is authorised
            set_caller::<DefaultEnvironment>(accounts.bob);
            az_airdrop.sub_admins_add(accounts.charlie).unwrap();
            set_caller::<DefaultEnvironment>(accounts.charlie);
            // = when airdrop has started
            ink::env::test::set_block_timestamp::<ink::env::DefaultEnvironment>(az_airdrop.start);
            // = * it raises an error
            result = az_airdrop.add_to_recipient(accounts.charlie, amount, None, None, None);
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

        #[ink::test]
        fn test_subtract_from_recipient() {
            let (accounts, mut az_airdrop) = init();
            let amount: Balance = 5;
            let recipient_address: AccountId = accounts.django;
            // when called by an admin or sub-admin
            // = when airdrop has started
            ink::env::test::set_block_timestamp::<ink::env::DefaultEnvironment>(az_airdrop.start);
            // = * it raises an error
            let mut result = az_airdrop.subtract_from_recipient(recipient_address, amount);
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
            result = az_airdrop.subtract_from_recipient(recipient_address, amount);
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
            result = az_airdrop.subtract_from_recipient(recipient_address, amount + 1);
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
                .subtract_from_recipient(recipient_address, amount - 1)
                .unwrap();
            let recipient: Recipient = az_airdrop.recipients.get(recipient_address).unwrap();
            assert_eq!(recipient.total_amount, 1);
            // when called by non-admin or non-sub-admin
            set_caller::<DefaultEnvironment>(accounts.charlie);
            // * it raises an error
            result = az_airdrop.subtract_from_recipient(recipient_address, amount);
            assert_eq!(result, Err(AzAirdropError::Unauthorised));
            // === * it reduces the total_amount
            assert_eq!(az_airdrop.to_be_collected, 1);
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

    // The main purpose of the e2e tests are to test the interactions with az groups contract
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
        async fn test_add_to_recipient(mut client: ::ink_e2e::Client<C, E>) -> E2EResult<()> {
            let bob_account_id: AccountId = account_id(ink_e2e::bob());

            // Instantiate token
            let token_constructor = ButtonRef::new(
                MOCK_AMOUNT,
                Some("Button".to_string()),
                Some("BTN".to_string()),
                6,
            );
            let token_id: AccountId = client
                .instantiate("az_button", &ink_e2e::alice(), token_constructor, 0, None)
                .await
                .expect("Token instantiate failed")
                .account_id;

            // Instantiate airdrop smart contract
            let default_collectable_at_tge_percentage: u8 = 20;
            let default_cliff_duration: Timestamp = 0;
            let default_vesting_duration: Timestamp = 31556952000;
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
            let add_to_recipient_message = build_message::<AzAirdropRef>(airdrop_id)
                .call(|airdrop| airdrop.add_to_recipient(bob_account_id, 1, None, None, None));
            let result = client
                .call_dry_run(&ink_e2e::alice(), &add_to_recipient_message, 0, None)
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
            let add_to_recipient_message = build_message::<AzAirdropRef>(airdrop_id)
                .call(|airdrop| airdrop.add_to_recipient(bob_account_id, 1, None, None, None));
            client
                .call(&ink_e2e::alice(), add_to_recipient_message, 0, None)
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
    }
}
