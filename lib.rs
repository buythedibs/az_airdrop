#![cfg_attr(not(feature = "std"), no_std, no_main)]

#[ink::contract]
mod az_airdrop {
    use ink::storage::Mapping;

    // === STRUCTS ===
    #[derive(Debug, Clone, scale::Encode, scale::Decode)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub struct Config {
        admin: AccountId,
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
        default_collectable_at_tge: Option<u8>,
        // ms from start user has to wait before either starting vesting, or collecting remaining available.
        default_cliff: Option<Timestamp>,
        // ms to collect all remaining after collection at tge
        default_vesting: Option<Timestamp>,
    }

    // === CONTRACT ===
    #[ink(storage)]
    pub struct AzAirdrop {
        admin: AccountId,
        start: Timestamp,
        recipients: Mapping<AccountId, Recipient>,
        default_collectable_at_tge: Option<u8>,
        default_cliff: Option<Timestamp>,
        default_vesting: Option<Timestamp>,
    }
    impl AzAirdrop {
        #[ink(constructor)]
        pub fn new(
            start: Timestamp,
            default_collectable_at_tge: Option<u8>,
            default_cliff: Option<Timestamp>,
            default_vesting: Option<Timestamp>,
        ) -> Self {
            Self {
                admin: Self::env().caller(),
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
                start: self.start,
                default_collectable_at_tge: self.default_collectable_at_tge,
                default_cliff: self.default_cliff,
                default_vesting: self.default_vesting,
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
            let az_airdrop = AzAirdrop::new(MOCK_START, None, None, None);
            (accounts, az_airdrop)
        }

        // === TESTS ===
        // === TEST QUERIES ===
        #[ink::test]
        fn test_config() {
            let (accounts, az_airdrop) = init();
            let config = az_airdrop.config();
            // * it returns the config
            assert_eq!(config.admin, accounts.bob);
            assert_eq!(config.start, MOCK_START);
            assert_eq!(config.default_collectable_at_tge, None);
            assert_eq!(config.default_cliff, None);
            assert_eq!(config.default_vesting, None);
        }
    }
}
