#![deny(unsafe_code)]
#![allow(clippy::new_without_default)]
#![cfg_attr(not(feature = "std"), no_std)]
//! ## Edgeware Tipping bot.
//!
//! Support developers work, and anyone who help you in the group chat with some
//! EDG tip. by simply sending `/tip @thier_username #EDG100` into the group
//! chat, and the bot will handle the rest, this would transfer 100 EDG from
//! your account to thier account.
//!
//! ### How it works?
//!
//! in simple steps, you chat with the bot, for the first time, it would ask you
//! to follow simple instructions on how to add the deployed contract to your
//! Polkadotjs Apps UI, by simply using the contract address and the ABI file.
//!
//! Next, you would call the `bind` function, allowing you to bind your telegram
//! account to your Edgeware idenity, the bot will provide you with your
//! telegram id (easy!). While calling this function, it _optionally_ accepts
//! some balance/EDG to be transfered from your account to the contract, saving
//! that as a balance to be used for tipping later.
//!
//! Now, with setup part done, you can now tip anyone by sending the `/tip`
//! command in the chat. if the recipient does not have a binding in the
//! contract, the bot will DM them to follow the above instructions, and will
//! reply to your message to send it again so they can get thier tip.
//! The bot does not hold thier funds in this case for security reasons^1.
//!
//! You can later `unbind` your account and any balance in the contract you own
//! will be refunded back to your account again.
//!
//! ^1: We don't hold the funds for the reason that someone else could bind
//! thier account to the recipient account, and claim it later, so we enforce
//! this role here that the target/recipient must be known thier account (they
//! must have a mapping between thier telegram id to an AccountId) in contract
//! to work properly.

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
            self.unbind_account(caller)
        }

        /// Similar to unbind, but only the owner can call this function.
        ///
        /// Errors:
        /// * Returns `Error::NotAllowed` if the caller is not the owner of the
        ///   contract.
        ///
        /// * Returns `Error::NotFound` if the caller's `AccountId` is not
        ///   bounded before.
        #[ink(message)]
        pub fn force_unbind(
            &mut self,
            account: AccountId,
        ) -> Result<(), Error> {
            self.ensure_owner()?;
            self.unbind_account(account)
        }

        /// Tip a Telegram user using thier `TelegramId`.
        ///
        /// This function should not be called directly by the user.
        /// Instead, the user should chat with the Telegram bot and the bot
        /// would call these function for them.
        /// Errors:
        /// * Returns `Error::NotFound` if the `tg_id` is not bounded to any
        ///   `AccountId`. also, if the caller is not bounded to any telegram
        ///   account.
        ///
        /// * Returns `Error::InsufficientFunds` when the caller does not have
        ///   enough balance.
        #[ink(message)]
        pub fn tip(
            &mut self,
            tg_id: TelegramId,
            amount: Balance,
        ) -> Result<(), Error> {
            let caller = self.env().caller();
            let inputs = self
                .telegram_id_of(Some(caller))
                .zip(self.address_of(tg_id));

            match inputs {
                Some((_, target)) => self.tip_account(caller, target, amount),
                None => panic_on_err!(Err(Error::NotFound)),
            }
        }

        /// Similar to tip, but only the owner can call this function.
        ///
        /// Called in behalf of the `from` TelegramId owner using the bot.
        ///
        /// Errors:
        /// * Returns `Error::NotAllowed` if the caller is not the owner of the
        ///   contract.
        ///
        /// * Returns `Error::NotFound` if the `from` or `to` is not bounded to
        ///   any telegram account.
        #[ink(message)]
        pub fn tip_from(
            &mut self,
            from: TelegramId,
            to: TelegramId,
            amount: Balance,
        ) -> Result<(), Error> {
            self.ensure_owner()?;
            let inputs = self.address_of(from).zip(self.address_of(to));
            match inputs {
                Some((from, to)) => self.tip_account(from, to, amount),
                None => panic_on_err!(Err(Error::NotFound)),
            }
        }

        fn unbind_account(&mut self, account: AccountId) -> Result<(), Error> {
            if !self.address_tg.contains_key(&account) {
                return panic_on_err!(Err(Error::NotFound));
            }
            let tg_id = self
                .address_tg
                .take(&account)
                .expect("the caller address exists");
            let _ = self
                .tg_address
                .take(&tg_id)
                .expect("the caller tg id exists");
            // if the caller have some balance, transfer it back to them.
            if let Some(v) = self.balances.take(&account) {
                return panic_on_err! {
                    self.env().transfer(account, v).map_err(|_| Error::BelowSubsistenceThreshold)
                };
            }
            Ok(())
        }

        fn tip_account(
            &mut self,
            caller: AccountId,
            target: AccountId,
            amount: Balance,
        ) -> Result<(), Error> {
            let balance = self.balances.get_mut(&caller);
            match balance {
                Some(value) if *value >= amount => {
                    *value -= amount;
                    panic_on_err! {
                        self
                            .env()
                            .transfer(target, amount)
                            .map_err(|_| Error::BelowSubsistenceThreshold)
                    }
                    // TODO(@shekohex): emit some events here.
                },
                Some(_) | None => panic_on_err!(Err(Error::InsufficientFunds)),
            }
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

        #[ink::test]
        #[should_panic(expected = "AlreadyBounded")]
        fn already_bounded() {
            set_from_owner();
            let mut bot = Tipbot::new();
            let accounts = default_accounts();
            // bind bob.
            set_sender(accounts.bob, 100);
            assert!(bot.bind(42).is_ok());
            set_sender(accounts.eve, 1);
            // 42 is already bounded.
            assert!(bot.bind(42).is_err());
        }

        #[ink::test]
        fn bind_twice() {
            set_from_owner();
            let mut bot = Tipbot::new();
            let accounts = default_accounts();
            // bind bob.
            set_sender(accounts.bob, 100);
            assert!(bot.bind(42).is_ok());
            assert_eq!(bot.address_of(42), Some(accounts.bob));
            // rebind to 4242.
            assert!(bot.bind(4242).is_ok());
            assert_eq!(bot.address_of(4242), Some(accounts.bob));
            // now 42 is gone.
            assert_eq!(bot.address_of(42), None);
        }

        #[ink::test]
        fn bind_twice_balance() {
            set_from_owner();
            let mut bot = Tipbot::new();
            let accounts = default_accounts();

            set_sender(accounts.bob, 100);
            assert!(bot.bind(42).is_ok());
            assert_eq!(bot.balance_of(42), 100);

            set_sender(accounts.bob, 200);
            assert!(bot.bind(4242).is_ok());
            assert_eq!(bot.balance_of(4242), 100 + 200);
        }

        #[ink::test]
        fn unbind_works() {
            let mut bot = create_contract(1000);
            let accounts = default_accounts();
            set_sender(accounts.bob, 100);

            assert!(bot.bind(42).is_ok());
            assert_eq!(bot.balance_of(42), 100);

            assert!(bot.unbind().is_ok());
            assert_eq!(bot.balance_of(42), 0);
        }

        #[ink::test]
        #[should_panic(expected = "NotFound")]
        fn unbind_not_found() {
            let mut bot = create_contract(1000);
            let accounts = default_accounts();
            set_sender(accounts.bob, 100);

            assert!(bot.unbind().is_err());
            assert_eq!(bot.balance_of(42), 0);
        }

        #[ink::test]
        fn force_unbind_works() {
            let mut bot = create_contract(1000);
            let accounts = default_accounts();
            set_sender(accounts.bob, 100);

            assert!(bot.bind(42).is_ok());
            assert_eq!(bot.balance_of(42), 100);

            set_from_owner();
            assert!(bot.force_unbind(accounts.bob).is_ok());
            assert_eq!(bot.balance_of(42), 0);
        }

        #[ink::test]
        #[should_panic(expected = "NotAllowed")]
        fn force_unbind_noowner() {
            let mut bot = create_contract(1000);
            let accounts = default_accounts();
            set_sender(accounts.bob, 100);

            assert!(bot.bind(42).is_ok());
            assert_eq!(bot.balance_of(42), 100);

            set_from_noowner();
            assert!(bot.force_unbind(accounts.bob).is_err());
            assert_eq!(bot.balance_of(42), 100);
        }

        #[ink::test]
        fn tipping_works() {
            let mut bot = create_contract(1000);
            let accounts = default_accounts();

            // bind alice account.
            set_sender(accounts.alice, 100);
            assert!(bot.bind(42).is_ok());
            assert_eq!(bot.balance_of(42), 100);

            // now bind bob account.
            set_sender(accounts.bob, 0);
            assert!(bot.bind(142).is_ok());
            assert_eq!(bot.balance_of(142), 0);
            set_balance(accounts.bob, 1); // set that they are have only 1 token.

            set_caller(accounts.alice);
            assert!(bot.tip(142, 50).is_ok()); // tip bob with 50.
            assert_eq!(bot.balance_of(42), 50); // now we have 50.
            assert_eq!(bot.balance_of(142), 0); // bob is still zero.
            assert_eq!(get_balance(accounts.bob), 51); // they have balance now.
        }

        #[ink::test]
        #[should_panic(expected = "NotFound")]
        fn tipping_not_found() {
            let mut bot = create_contract(1000);
            let accounts = default_accounts();

            // bind alice account.
            set_sender(accounts.alice, 100);
            assert!(bot.bind(42).is_ok());
            assert_eq!(bot.balance_of(42), 100);

            set_caller(accounts.alice);
            assert!(bot.tip(142, 50).is_err()); // tip `142` with 50.
            assert_eq!(bot.balance_of(42), 100); // still 100.
        }

        #[ink::test]
        #[should_panic(expected = "InsufficientFunds")]
        fn tipping_no_balance() {
            let mut bot = create_contract(1000);
            let accounts = default_accounts();

            // bind alice account.
            set_sender(accounts.alice, 100);
            assert!(bot.bind(42).is_ok());
            assert_eq!(bot.balance_of(42), 100);
            // now bind bob account.
            set_sender(accounts.bob, 0);
            assert!(bot.bind(142).is_ok());

            set_caller(accounts.alice);
            assert!(bot.tip(142, 150).is_err()); // tip bob with 150.
            assert_eq!(bot.balance_of(42), 100); // still 100.
        }

        fn create_contract(initial_balance: Balance) -> Tipbot {
            set_from_owner();
            set_balance(contract_id(), initial_balance);
            Tipbot::new()
        }

        fn set_caller(account: AccountId) { set_sender(account, 100_000); }

        fn set_sender(sender: AccountId, endowment: Balance) {
            let mut data = test::CallData::new(call::Selector::new([0x00; 4]));
            data.push_arg(&sender);
            test::push_execution_context::<Environment>(
                sender,
                contract_id(),
                1000000,   // gas
                endowment, // endowment
                data,
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

        fn contract_id() -> AccountId {
            test::get_current_contract_account_id::<ink_env::DefaultEnvironment>(
            )
            .expect("Cannot get contract id")
        }

        fn set_balance(account_id: AccountId, balance: Balance) {
            test::set_account_balance::<ink_env::DefaultEnvironment>(
                account_id, balance,
            )
            .expect("Cannot set account balance");
        }

        fn get_balance(account_id: AccountId) -> Balance {
            test::get_account_balance::<ink_env::DefaultEnvironment>(account_id)
                .expect("Cannot set account balance")
        }
    }
}
