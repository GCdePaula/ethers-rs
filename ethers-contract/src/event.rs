use crate::{stream::EventStream, ContractError, EthLogDecode};

use ethers_core::{
    abi::{Detokenize, RawLog},
    types::{BlockNumber, Filter, Log, TxHash, ValueOrArray, H256, U64, U256, Address},
};
use ethers_providers::{FilterWatcher, Middleware, PubsubClient, SubscriptionStream};
use std::borrow::Cow;
use std::marker::PhantomData;

/// A trait for implementing event bindings
pub trait EthEvent: Detokenize {
    /// The name of the event this type represents
    fn name() -> Cow<'static, str>;

    /// Retrieves the signature for the event this data corresponds to.
    /// This signature is the Keccak-256 hash of the ABI signature of
    /// this event.
    fn signature() -> H256;

    /// Retrieves the ABI signature for the event this data corresponds
    /// to.
    fn abi_signature() -> Cow<'static, str>;

    /// Decodes an Ethereum `RawLog` into an instance of the type.
    fn decode_log(log: &RawLog) -> Result<Self, ethers_core::abi::Error>
    where
        Self: Sized;

    /// Returns true if this is an anonymous event
    fn is_anonymous() -> bool;

    /// Returns an Event builder for the ethereum event represented by this types ABI signature.
    fn new<M: Middleware>(filter: Filter, provider: &M) -> Event<M, Self>
    where
        Self: Sized,
    {
        let filter = filter.event(&Self::abi_signature());
        Event {
            filter,
            provider,
            datatype: PhantomData,
        }
    }
}

// Convenience implementation
impl<T: EthEvent> EthLogDecode for T {
    fn decode_log(log: &RawLog) -> Result<Self, ethers_core::abi::Error>
    where
        Self: Sized,
    {
        T::decode_log(log)
    }
}

/// Helper for managing the event filter before querying or streaming its logs
#[derive(Debug)]
#[must_use = "event filters do nothing unless you `query` or `stream` them"]
pub struct Event<'a, M, D> {
    /// The event filter's state
    pub filter: Filter,
    pub(crate) provider: &'a M,
    /// Stores the event datatype
    pub(crate) datatype: PhantomData<D>,
}

// TODO: Improve these functions
impl<M, D: EthLogDecode> Event<'_, M, D> {
    /// Sets the filter's `from` block
    #[allow(clippy::wrong_self_convention)]
    pub fn from_block<T: Into<BlockNumber>>(mut self, block: T) -> Self {
        self.filter = self.filter.from_block(block);
        self
    }

    /// Sets the filter's `to` block
    #[allow(clippy::wrong_self_convention)]
    pub fn to_block<T: Into<BlockNumber>>(mut self, block: T) -> Self {
        self.filter = self.filter.to_block(block);
        self
    }

    /// Sets the filter's `blockHash`. Setting this will override previously
    /// set `from_block` and `to_block` fields.
    #[allow(clippy::wrong_self_convention)]
    pub fn at_block_hash<T: Into<H256>>(mut self, hash: T) -> Self {
        self.filter = self.filter.at_block_hash(hash);
        self
    }

    /// Sets the filter's 0th topic (typically the event name for non-anonymous events)
    pub fn topic0<T: Into<ValueOrArray<H256>>>(mut self, topic: T) -> Self {
        self.filter.topics[0] = Some(topic.into());
        self
    }

    /// Sets the filter's 1st topic
    pub fn topic1<T: Into<ValueOrArray<H256>>>(mut self, topic: T) -> Self {
        self.filter.topics[1] = Some(topic.into());
        self
    }

    /// Sets the filter's 2nd topic
    pub fn topic2<T: Into<ValueOrArray<H256>>>(mut self, topic: T) -> Self {
        self.filter.topics[2] = Some(topic.into());
        self
    }

    /// Sets the filter's 3rd topic
    pub fn topic3<T: Into<ValueOrArray<H256>>>(mut self, topic: T) -> Self {
        self.filter.topics[3] = Some(topic.into());
        self
    }
}

impl<'a, M, D> Event<'a, M, D>
where
    M: Middleware,
    D: EthLogDecode,
{
    /// Returns a stream for the event
    pub async fn stream(
        &'a self,
    ) -> Result<
        // Wraps the FilterWatcher with a mapping to the event
        EventStream<'a, FilterWatcher<'a, M::Provider, Log>, D, ContractError<M>>,
        ContractError<M>,
    > {
        let filter = self
            .provider
            .watch(&self.filter)
            .await
            .map_err(ContractError::MiddlewareError)?;
        Ok(EventStream::new(
            filter.id,
            filter,
            Box::new(move |log| self.parse_log(log)),
        ))
    }
}

impl<'a, M, D> Event<'a, M, D>
where
    M: Middleware,
    <M as Middleware>::Provider: PubsubClient,
    D: EthLogDecode,
{
    /// Returns a subscription for the event
    pub async fn subscribe(
        &'a self,
    ) -> Result<
        // Wraps the SubscriptionStream with a mapping to the event
        EventStream<'a, SubscriptionStream<'a, M::Provider, Log>, D, ContractError<M>>,
        ContractError<M>,
    > {
        let filter = self
            .provider
            .subscribe_logs(&self.filter)
            .await
            .map_err(ContractError::MiddlewareError)?;
        Ok(EventStream::new(
            filter.id,
            filter,
            Box::new(move |log| self.parse_log(log)),
        ))
    }
}

impl<M, D> Event<'_, M, D>
where
    M: Middleware,
    D: EthLogDecode,
{
    /// Queries the blockchain for the selected filter and returns a vector of matching
    /// event logs
    pub async fn query(&self) -> Result<Vec<D>, ContractError<M>> {
        let logs = self
            .provider
            .get_logs(&self.filter)
            .await
            .map_err(ContractError::MiddlewareError)?;
        let events = logs
            .into_iter()
            .map(|log| self.parse_log(log))
            .collect::<Result<Vec<_>, ContractError<M>>>()?;
        Ok(events)
    }

    /// Queries the blockchain for the selected filter and returns a vector of logs
    /// along with their metadata
    pub async fn query_with_meta(&self) -> Result<Vec<(D, LogMeta)>, ContractError<M>> {
        let logs = self
            .provider
            .get_logs(&self.filter)
            .await
            .map_err(ContractError::MiddlewareError)?;
        let events = logs
            .into_iter()
            .map(|log| {
                let meta = LogMeta::from(&log);
                let event = self.parse_log(log)?;
                Ok((event, meta))
            })
            .collect::<Result<_, ContractError<M>>>()?;
        Ok(events)
    }

    fn parse_log(&self, log: Log) -> Result<D, ContractError<M>> {
        D::decode_log(&RawLog {
            topics: log.topics,
            data: log.data.to_vec(),
        })
        .map_err(From::from)
    }
}

/// Metadata inside a log
#[derive(Clone, Debug, PartialEq)]
pub struct LogMeta {
    /// Address from which this log originated
    pub address: Address,

    /// The block in which the log was emitted
    pub block_number: U64,

    /// The block hash in which the log was emitted
    pub block_hash: H256,

    /// The transaction hash in which the log was emitted
    pub transaction_hash: TxHash,

    /// Transactions index position log was created from
    pub transaction_index: U64,

    /// Log index position in the block
    pub log_index: U256,
}

impl From<&Log> for LogMeta {
    fn from(src: &Log) -> Self {
        LogMeta {
            address: src.address,
            block_number: src.block_number.expect("should have a block number"),
            block_hash: src.block_hash.expect("should have a block hash"),
            transaction_hash: src.transaction_hash.expect("should have a tx hash"),
            transaction_index: src.transaction_index.expect("should have a tx index"),
            log_index: src.log_index.expect("should have a log index"),
        }
    }
}
