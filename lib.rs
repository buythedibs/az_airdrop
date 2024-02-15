#![cfg_attr(not(feature = "std"), no_std, no_main)]

#[ink::contract]
mod az_airdrop {
    // === STRUCTS ===
    #[derive(Debug, Clone, scale::Encode, scale::Decode)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub struct Config {
        admin: AccountId,
    }

    // === CONTRACT ===
    #[ink(storage)]
    pub struct AzAirdrop {
        admin: AccountId,
    }
    impl AzAirdrop {
        #[ink(constructor)]
        pub fn new() -> Self {
            Self {
                admin: Self::env().caller(),
            }
        }

        // === QUERIES ===
        #[ink(message)]
        pub fn config(&self) -> Config {
            Config { admin: self.admin }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use ink::env::{
            test::{default_accounts, set_caller, DefaultAccounts},
            DefaultEnvironment,
        };

        // === HELPERS ===
        fn init() -> (DefaultAccounts<DefaultEnvironment>, AzAirdrop) {
            let accounts = default_accounts();
            set_caller::<DefaultEnvironment>(accounts.bob);
            let az_airdrop = AzAirdrop::new();
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
        }
    }
}
