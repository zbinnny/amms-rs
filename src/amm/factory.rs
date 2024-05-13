use std::cmp::min;
use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::sync::Arc;

use async_trait::async_trait;
use ethers::{
    providers::{Middleware, StreamExt},
    types::{Filter, Log, H160, H256},
};
use futures::stream::FuturesUnordered;
use serde::{Deserialize, Serialize};

use crate::errors::{AMMError, EventLogError};

use super::{
    uniswap_v2::factory::{UniswapV2Factory, PAIR_CREATED_EVENT_SIGNATURE},
    AMM,
};

#[async_trait]
pub trait AutomatedMarketMakerFactory {
    /// Returns the address of the factory.
    fn address(&self) -> H160;

    /// Returns the block number at which the factory was created.
    fn creation_block(&self) -> u64;

    /// Returns the creation event signature for the factory.
    fn amm_created_event_signature(&self) -> H256;

    /// Creates a new empty AMM from a log factory creation event.
    fn new_empty_amm_from_log(&self, log: Log) -> Result<AMM, ethers::abi::Error>;
}

macro_rules! factory {
    ($($factory_type:ident),+ $(,)?) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum Factory {
            $($factory_type($factory_type),)+
        }

        #[async_trait]
        impl AutomatedMarketMakerFactory for Factory {
            fn address(&self) -> H160 {
                match self {
                    $(Factory::$factory_type(factory) => factory.address(),)+
                }
            }

            fn creation_block(&self) -> u64 {
                match self {
                    $(Factory::$factory_type(factory) => factory.creation_block(),)+
                }
            }

            fn amm_created_event_signature(&self) -> H256 {
                match self {
                    $(Factory::$factory_type(factory) => factory.amm_created_event_signature(),)+
                }
            }

            fn new_empty_amm_from_log(&self, log: Log) -> Result<AMM, ethers::abi::Error> {
                match self {
                    $(Factory::$factory_type(factory) => factory.new_empty_amm_from_log(log),)+
                }
            }
        }
    };
}

factory!(UniswapV2Factory);

impl TryFrom<H256> for Factory {
    type Error = EventLogError;

    fn try_from(value: H256) -> Result<Self, Self::Error> {
        if value == PAIR_CREATED_EVENT_SIGNATURE {
            Ok(Factory::UniswapV2Factory(UniswapV2Factory::default()))
        } else {
            return Err(EventLogError::InvalidEventSignature);
        }
    }
}

pub struct FactoryHelper {
    factories: HashMap<H160, Factory>,
}

impl FactoryHelper {
    pub fn new(factories: HashMap<H160, Factory>) -> Self {
        FactoryHelper { factories }
    }

    // amm建立事件过滤器
    fn amm_created_event_filter(&self) -> Filter {
        let mut event_signatures = vec![];
        let mut factories_set = HashSet::new();

        for (_, factory) in self.factories.iter() {
            let address = factory.address();
            if factories_set.contains(&address) {
                continue;
            }

            factories_set.insert(address);
            event_signatures.push(factory.amm_created_event_signature());
        }

        Filter::new().topic0(event_signatures)
    }

    pub async fn get_empty_pools_from_logs<M: 'static + Middleware>(
        &self,
        mut from_block: u64,
        to_block: Option<u64>,
        step: u64,
        middleware: Arc<M>,
    ) -> Result<(Vec<AMM>, u64), AMMError<M>> {
        let to_block = match to_block {
            None => middleware
                .get_block_number()
                .await
                .map_err(AMMError::MiddlewareError)?
                .as_u64(),
            Some(block) => block,
        };

        let filter = self.amm_created_event_filter().address(
            self.factories
                .values()
                .clone()
                .map(|f| f.address())
                .collect::<Vec<H160>>(),
        );

        // 初始化并行任务
        let mut futures = FuturesUnordered::new();
        while from_block < to_block {
            let middleware = middleware.clone();
            let target_block = min(from_block + step - 1, to_block);

            let filter = filter.clone().from_block(from_block).to_block(target_block);
            futures.push(async move { middleware.get_logs(&filter).await });

            from_block += step;
        }

        let mut aggregated_amms = vec![];

        while let Some(result) = futures.next().await {
            let logs = result.map_err(AMMError::MiddlewareError)?;

            for log in logs {
                let factory = match self.factories.get(&log.address) {
                    None => continue,
                    Some(v) => v,
                };

                let amm = match factory.new_empty_amm_from_log(log) {
                    Ok(amm) => amm,
                    Err(_) => continue,
                };

                aggregated_amms.push(amm);
            }
        }

        Ok((aggregated_amms, to_block))
    }
}
