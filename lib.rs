#![deny(unsafe_code)]
#![allow(clippy::new_without_default)]
#![cfg_attr(not(feature = "std"), no_std)]

use ink_lang as ink;

/// A macro to panic and abort any changes to the contract
/// on error.
/// Should be removed after this issue is resolved.
/// https://github.com/paritytech/ink/issues/745
macro_rules! panic_on_err {
    ($result: expr) => {
        match $result {
            Ok(v) => Result::<_, Error>::Ok(v),
            Err(e) => panic!("{:?}", e),
        }
    };
}

#[ink::contract]
mod tipbot {
    /// A Telegram User Id.
    type TelegramId = u32;

    /// Edgeware Tipping Bot
    #[ink(storage)]
    pub struct Tipbot {
        /// The contract owner, set to the account who deployed the contract
        owner: AccountId,
        address_tg: ink_storage::collections::HashMap<AccountId, TelegramId>,
        tg_address: ink_storage::collections::HashMap<TelegramId, AccountId>,
        balances: ink_storage::collections::HashMap<AccountId, Balance>,
    }

    /// The Error cases.
    #[derive(Debug, PartialEq, Eq, scale::Encode)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub enum Error {
        /// Returned if the AccountId is already bounded to a TelegramId.
        AlreadyBounded,
        /// Returned if the caller is not the owner of the contract.
        NotAllowed,
        /// Returned if the address/telegram id is not found.
        NotFound,
        /// Returned if the transfer failed.
        TransferFailed,
        /// Insufficient funds to execute transfer.
        InsufficientFunds,
        /// Transfer failed because it would have brought the contract's
        /// balance below the subsistence threshold.
        /// This is necessary to keep enough funds in the contract to
        /// allow for a tombstone to be created.
        BelowSubsistenceThreshold,
    }

    impl Tipbot {
        /// Create new Tipbot.
        /// The `caller` of this constructor will be set as the Owner of the
        /// contract.
        #[ink(constructor)]
        pub fn new() -> Self {
            Self {
                owner: Self::env().caller(),
                address_tg: Default::default(),
                tg_address: Default::default(),
                balances: Default::default(),
            }
        }

        /// Query for the Telegram Id of some account.
        /// if the account is not provided, will return the telegram id of the
        /// caller.
        #[ink(message)]
        pub fn telegram_id_of(
            &self,
            account: Option<AccountId>,
        ) -> Option<TelegramId> {
            let address = account.unwrap_or_else(|| self.env().caller());
            self.address_tg.get(&address).cloned()
        }

        /// Query The AccountId of the TelegramId.
        #[ink(message)]
        pub fn address_of(&self, tg_id: TelegramId) -> Option<AccountId> {
            self.tg_address.get(&tg_id).cloned()
        }

        /// Query The Balance of the TelegramId.
        #[ink(message)]
        pub fn balance_of(&self, tg_id: TelegramId) -> Balance {
            if let Some(address) = self.address_of(tg_id) {
                self.balances.get(&address).cloned().unwrap_or(0)
            } else {
                0
            }
        }

        /// Bind the caller address to the provided TelegramId.
        ///
        /// Errors:
        /// Returns `Error::AlreadyBounded` if the AccountId is already bounded
        /// to a TelegramId.
        #[ink(message, payable)]
        pub fn bind(&mut self, tg_id: TelegramId) -> Result<(), Error> {
            // if we already know this return an error, to prevent from
            // account spoofing.
            if self.tg_address.contains_key(&tg_id) {
                return panic_on_err!(Err(Error::AlreadyBounded));
            }
            let caller = self.env().caller();
            // add the new binding.
            if let Some(old_caller) = self.tg_address.insert(tg_id, caller) {
                // remove the old caller from the addresses.
                let _ = self.address_tg.take(&old_caller);
            }

            if let Some(old_tg_id) = self.address_tg.insert(caller, tg_id) {
                // free the old tg_id.
                //
                // this ensures that we always have one address for one telegram
                // id.
                let _ = self.tg_address.take(&old_tg_id);
            }
            // check if the user added some balance to thier account during the
            // call.
            let balance = self.env().transferred_balance();
            if balance > 0 {
                self.balances
                    .entry(caller)
                    .and_modify(|v| *v += balance)
                    .or_insert(balance);
            }
            Ok(())
        }

        /// Unbind the caller address from thier telegram account.
        /// and _optionally_ transfer any balance if they have any.
        ///
        /// Errors:
        /// Returns `Error::NotFound` if the caller's `AccountId` is not bounded
        /// before.
        #[ink(message)]
        pub fn unbind(&mut self) -> Result<(), Error> {
            let caller = self.env().caller();
            if !self.address_tg.contains_key(&caller) {
                return panic_on_err!(Err(Error::NotFound));
            }
            let tg_id = self
                .address_tg
                .take(&caller)
                .expect("the caller address exists");
            let _ = self
                .tg_address
                .take(&tg_id)
                .expect("the caller tg id exists");
            // if the caller have some balance, transfer it back to them.
            if let Some(v) = self.balances.take(&caller) {
                return panic_on_err! {
                    self.env().transfer(caller, v).map_err(|_| Error::BelowSubsistenceThreshold)
                };
            }
            Ok(())
        }

        /// Ensures that the caller is the owner of the contract.
        /// otherwise, returns `Error::NotAllowed`.
        #[inline(always)]
        fn ensure_owner(&self) -> Result<(), Error> {
            panic_on_err! {
                self.env().caller().eq(&self.owner).then(|| ()).ok_or(Error::NotAllowed)
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use ink_env::{call, test};
        use ink_lang as ink;

        type Accounts = test::DefaultAccounts<Environment>;

        #[ink::test]
        fn happy_path() {
            set_from_owner();
            let mut bot = Tipbot::new();
            let accounts = default_accounts();

            // ensures everything is clean.
            assert_eq!(bot.telegram_id_of(Some(accounts.bob)), None);
            assert_eq!(bot.balance_of(42), 0);
            // bind bob.
            set_sender(accounts.bob, 6969);
            assert!(bot.bind(42).is_ok());

            assert_eq!(bot.telegram_id_of(Some(accounts.bob)), Some(42));
            assert_eq!(bot.address_of(42), Some(accounts.bob));
            assert_eq!(bot.balance_of(42), 6969);
        }

        fn set_caller(account: AccountId) { set_sender(account, 100_000); }

        fn set_sender(sender: AccountId, endowment: Balance) {
            test::push_execution_context::<Environment>(
                sender,
                [42u8; 32].into(),
                1000000,   // gas
                endowment, // endowment
                test::CallData::new(call::Selector::new([0x00; 4])), // dummy
            );
        }

        fn set_from_owner() {
            let accounts = default_accounts();
            set_caller(accounts.alice);
        }

        fn set_from_noowner() {
            let accounts = default_accounts();
            set_caller(accounts.django);
        }

        fn default_accounts() -> Accounts {
            test::default_accounts()
                .expect("Test environment is expected to be initialized.")
        }
    }
}
