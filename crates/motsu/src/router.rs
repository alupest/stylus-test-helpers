//! Router context for external calls mocks.

use std::{
    borrow::BorrowMut, marker::PhantomData, sync::LazyLock, thread::ThreadId,
};

use alloy_primitives::Address;
use dashmap::{mapref::one::RefMut, DashMap};
use stylus_sdk::{
    abi::router_entrypoint,
    host::{WasmVM, VM},
    prelude::{StorageType, TopLevelStorage, ValueDenier},
    ArbResult,
};

use crate::{
    context::create_default_storage_type, storage_access::AccessStorage,
};

/// Motsu VM Router Storage.
///
/// A global mutable key-value store that allows concurrent access.
///
/// The key is the [`VMRouter`], a combination of [`ThreadId`] and
/// [`Address`] to avoid a panic on lock, while calling more than two contracts
/// consecutive.
///
/// The value is the [`VMRouterStorage`], a router of the contract generated by
/// `stylus-sdk`.
///
/// NOTE: The [`VMRouter::storage`] will panic on lock, when the same key
/// is accessed twice from the same thread.
static MOTSU_VM_ROUTERS: LazyLock<DashMap<VMRouter, VMRouterStorage>> =
    LazyLock::new(DashMap::new);

/// Context of Motsu test VM router associated with the current test thread and
/// contract's address.
#[derive(Hash, Eq, PartialEq, Copy, Clone)]
pub(crate) struct VMRouter {
    thread_id: ThreadId,
    contract_address: Address,
}

impl VMRouter {
    /// Create a new router context.
    pub(crate) fn new(thread: ThreadId, contract_address: Address) -> Self {
        Self { thread_id: thread, contract_address }
    }

    /// Get reference to the call storage for the current test thread.
    fn storage(self) -> RefMut<'static, VMRouter, VMRouterStorage> {
        MOTSU_VM_ROUTERS.access_storage(&self)
    }

    /// Check if the router exists for the contract.
    pub(crate) fn exists(self) -> bool {
        MOTSU_VM_ROUTERS.contains_key(&self)
    }

    pub(crate) fn route(self, calldata: Vec<u8>) -> ArbResult {
        let storage = self.storage();
        let mut router = storage.router_factory.create();

        // Drop the storage reference to avoid a panic on lock.
        drop(storage);

        router.route(calldata)
    }

    /// Initialise contract router for the current test thread and
    /// `contract_address`.
    pub(crate) fn init_storage<ST: StorageType + Router + 'static>(self) {
        let contract_address = self.contract_address;
        if MOTSU_VM_ROUTERS
            .insert(
                self,
                VMRouterStorage {
                    router_factory: Box::new(RouterFactory::<ST> {
                        phantom: PhantomData,
                    }),
                },
            )
            .is_some()
        {
            panic!("contract's router is already initialized - contract_address is {contract_address}");
        }
    }

    /// Reset router storage for the current [`VMRouter`].
    pub(crate) fn reset_storage(self) {
        MOTSU_VM_ROUTERS.remove(&self);
    }
}

/// Metadata related to the router of an external contract.
struct VMRouterStorage {
    // Contract's router.
    router_factory: Box<dyn CreateRouter>,
}

/// A trait for router's creation.
trait CreateRouter: Send + Sync {
    /// Instantiate a new router.
    fn create(&self) -> Box<dyn Router>;
}

/// A factory for router creation.
struct RouterFactory<R> {
    phantom: PhantomData<R>,
}

// SAFETY: We used `PhantomData` and lied to rust compiler that
// [`RouterFactory`] contains type `R`.
// In fact, it is a void type that contains neither other types nor references
// and can be safely shared or sent between threads.
// We will cheat rust the second time and explicitly implement `Send` and `Sync`
// for [`RouterFactory`].
unsafe impl<R> Send for RouterFactory<R> {}
unsafe impl<R> Sync for RouterFactory<R> {}

impl<R: StorageType + Router + 'static> CreateRouter for RouterFactory<R> {
    fn create(&self) -> Box<dyn Router> {
        Box::new(create_default_storage_type::<R>())
    }
}

/// A trait for routing messages to the matching selector.
#[allow(clippy::module_name_repetitions)]
pub trait Router {
    /// Tries to find and execute a method for the given `selector`, returning
    /// `None` if the `selector` wasn't found.
    fn route(&mut self, calldata: Vec<u8>) -> ArbResult;
}

impl<R> Router for R
where
    R: stylus_sdk::abi::Router<R>
        + StorageType
        + TopLevelStorage
        + BorrowMut<R::Storage>
        + ValueDenier,
{
    fn route(&mut self, calldata: Vec<u8>) -> ArbResult {
        router_entrypoint::<Self, Self>(
            calldata,
            VM { host: Box::new(WasmVM {}) },
        )
    }
}
