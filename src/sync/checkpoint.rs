use std::{
    cmp::min,
    collections::{HashMap, HashSet},
    fs::read_to_string,
    sync::Arc,
};
use std::fmt::{Display, Formatter};

use ethers::{
    prelude::{Filter, H160},
    providers::Middleware,
};
use ethers::prelude::{Address, Log, StreamExt};
use futures::stream::FuturesUnordered;
use serde::{Deserialize, Serialize};

use crate::{
    amm::{
        AMM,
        AutomatedMarketMaker,
        factory::{AutomatedMarketMakerFactory, FactoryHelper}, factory::Factory,
    },
    currency::{batch_get_currency_info, Currency},
    errors::{AMMError, CheckpointError, EventLogError},
    sync::serde_with::*,
};
use crate::amm::amm_sync_event_signatures;

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Checkpoint {
    pub block_number: Option<u64>,
    #[serde(
        serialize_with = "serialize_map_to_vec",
        deserialize_with = "deserialize_vec_to_map"
    )]
    pub factories: HashMap<H160, Factory>,
    #[serde(
        serialize_with = "serialize_map_to_vec",
        deserialize_with = "deserialize_vec_to_map"
    )]
    pub amms: HashMap<H160, AMM>,
    #[serde(
        serialize_with = "serialize_map_to_vec",
        deserialize_with = "deserialize_vec_to_map"
    )]
    pub currencies: HashMap<H160, Currency>,
    // 货币黑名单, 用于过滤掉无效的货币
    pub currencies_blacklist: HashSet<Address>,
}

impl Display for Checkpoint {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let last_synced_log_block = self.amms.iter().map(|(_, amm)| amm.last_synced_log().0).max().unwrap_or(0);
        write!(f, "Checkpoint(block_number: {}, factories: {}, amms: {}, invalid_amms: {}, currencies: {}, currencies_blacklist: {}, last_synced_log: {})",
               self.block_number.unwrap_or_default(),
               self.factories.len(),
               self.amms.len(),
               self.amms.iter().filter(|(_, amm)| amm.last_synced_log().0 == 0).count(),
               self.currencies.len(),
               self.currencies_blacklist.len(),
               last_synced_log_block,
        )
    }
}

impl Checkpoint {
    pub fn new_from_factories(factories: HashMap<H160, Factory>) -> Checkpoint {
        Checkpoint {
            factories,
            ..Default::default()
        }
    }

    /// 从文件创建新的 Checkpoint
    pub fn new_from_file(path: &str) -> Result<Checkpoint, CheckpointError> {
        let checkpoint: Checkpoint = serde_json::from_str(read_to_string(path)?.as_str())?;
        Ok(checkpoint)
    }

    /// 保存到文件
    pub fn save_to_file(&self, path: &str) -> Result<(), CheckpointError> {
        std::fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    /// 合并Checkpoint
    pub fn extend(&mut self, other: Checkpoint) {
        self.block_number = min(self.block_number, other.block_number);
        self.factories.extend(other.factories);
        self.amms.extend(other.amms);
    }

    pub fn block_number(&self) -> u64 {
        match self.block_number {
            None => self
                .factories
                .iter()
                .map(|(_, factory)| factory.creation_block())
                .min()
                .unwrap_or_default(), // 区块号为空时从工厂创建的区块开始
            Some(block) => block,
        }
    }

    /// 最后同步的日志所在的区块, 如果不存在则返回Checkpoint中最早的工厂创建区块
    pub fn last_synced_log_block(&self) -> u64 {
        let last_synced_block = self
            .amms
            .iter()
            .map(|(_, amm)| amm.last_synced_log().0)
            .max()
            .unwrap_or(0);


        if last_synced_block == 0 {
            self.factories
                .iter()
                .map(|(_, factory)| factory.creation_block())
                .min()
                .unwrap_or_default()
        } else {
            last_synced_block
        }
    }

    /// 删除包含无效货币的amm
    fn remove_invalid_amm(&mut self) {
        if self.currencies_blacklist.is_empty() {
            return;
        }

        // 找出黑名单中的货币对应的交易对
        let mut amms_to_remove = vec![];
        for (address, amm) in self.amms.iter() {
            let tokens = amm.tokens();

            for token in tokens.iter() {
                if self.currencies_blacklist.contains(token) {
                    amms_to_remove.push(address.clone());
                    break;
                }
            }
        }

        // 删除无效交易对
        for address in amms_to_remove {
            self.amms.remove(&address);
        }
    }

    /// 查找新的amms池子
    pub async fn find_new_amms<M: 'static + Middleware>(
        &mut self,
        middleware: Arc<M>,
    ) -> Result<(), AMMError<M>> {
        // 加载新的amms
        let start_block = self.block_number();
        tracing::info!(
            "根据factory合约地址加载新的amms池子. start_block: {}",
            start_block
        );
        let (new_amms, end_block) = FactoryHelper::new(self.factories.clone())
            .get_empty_pools_from_logs(start_block, None, 1000, middleware.clone())
            .await?;

        // 更新 checkpoint 数据
        let mut new_amm_count = 0;
        for amm in new_amms.into_iter() {
            // 跳过已经存在于 checkpoint 中的 amm
            if self.amms.contains_key(&amm.address()) {
                continue;
            }

            self.amms.insert(amm.address(), amm);
            new_amm_count += 1;
        }
        self.block_number = Some(end_block);
        tracing::info!(
            "更新池子数据. start: {}, end: {}, amms_count: {}",
            start_block,
            end_block,
            new_amm_count
        );

        Ok(())
    }

    pub async fn sync_currencies<M: 'static + Middleware>(
        &mut self,
        middleware: Arc<M>,
    ) -> Result<(), AMMError<M>> {
        let mut missing_currencies = HashSet::new();
        for (_, amm) in self.amms.iter_mut() {
            for token in amm.tokens().iter() {
                match self.currencies.get(token) {
                    None => {
                        // 收集缺失的currency
                        missing_currencies.insert(token.clone());
                    }
                    Some(currency) => amm.set_currency(currency.clone()), // 更新amm的currency信息
                }
            }
        }

        let step = 100usize;
        // 加载缺失的currency信息
        tracing::info!(
            "加载缺失的currency信息. 共缺少{}个",
            missing_currencies.len()
        );
        let currencies = batch_get_currency_info(
            missing_currencies.clone().into_iter().collect(),
            Some(step),
            middleware.clone(),
        ).await?;
        tracing::info!("加载缺失的currency信息完成. 共加载{}个", currencies.len());

        if currencies.is_empty() {
            return Ok(());
        }

        for currency in currencies {
            if currency.is_invalid_token() {
                continue;
            }

            self.currencies.insert(currency.address(), currency);
        }

        // 当step为1时, 说明是一个一个代币查询, 如果还报错，就是有坑人的交易对，直接将货币拉进黑名单, 并删除amm
        if step == 1 {
            for missing_currency in missing_currencies {
                if !self.currencies.contains_key(&missing_currency) {
                    self.currencies_blacklist.insert(missing_currency);
                }
            }

            // 更新黑名单后, 删除无效的amm
            self.remove_invalid_amm();
        }

        // 再次填充amm的currency信息
        for (_, amm) in self.amms.iter_mut() {
            for currency in amm.currencies().iter() {
                if currency.data_is_populated() {
                    continue;
                }

                match self.currencies.get(&currency.address()) {
                    None => continue,
                    Some(currency) => amm.set_currency(currency.clone()), // 更新amm的currency信息
                }
            }
        }

        Ok(())
    }

    /// 同步amms的深度数据
    pub async fn sync_amms_reserve<M: 'static + Middleware>(
        &mut self,
        middleware: Arc<M>,
    ) -> Result<(), AMMError<M>> {
        let latest_block = middleware
            .get_block_number()
            .await
            .map_err(AMMError::MiddlewareError)?
            .as_u64();

        // 创建事件过滤器
        let event_signatures = amm_sync_event_signatures(&self.amms);
        let block_filter = Filter::new().topic0(event_signatures);

        let mut start_block = self.last_synced_log_block() + 1; // +1, 因为当前块已经同步过了, 从下一个块开始查起
        let step = 2500u64;

        loop {
            if start_block >= latest_block {
                break;
            }

            let target_block = min(start_block + step, latest_block);

            tracing::info!(
                "Syncing state space from block {} to block {}, latest_block {}",
                start_block,
                target_block,
                latest_block
            );
            let logs = batch_request_logs(middleware.clone(), block_filter.clone(), start_block, target_block).await?; // 请求日志

            for log in logs {
                // 检查是否是状态空间中的amm的日志
                if let Some(amm) = self.amms.get_mut(&log.address) {
                    match amm.sync_from_log(log) {
                        Ok(_) => {}
                        Err(EventLogError::LogAlreadySynced) => continue,
                        Err(err) => return Err(AMMError::EventLogError(err)),
                    }
                }
            }

            start_block = target_block + 1;
        }

        Ok(())
    }
}


async fn batch_request_logs<M: 'static + Middleware>(
    middleware: Arc<M>,
    filter: Filter,
    start_block: u64,
    end_block: u64,
) -> Result<Vec<Log>, AMMError<M>> {

    // 初始化并发任务
    let mut futures = FuturesUnordered::new();
    let step = 250;
    for i in (start_block..=end_block).step_by(step) {
        let filter = filter.clone().from_block(i).to_block(min(i + step as u64, end_block));
        let middleware = middleware.clone();

        futures.push(async move { middleware.get_logs(&filter).await.map_err(AMMError::MiddlewareError) });
    }

    // 并发请求日志
    let mut logs = HashMap::new();

    while let Some(result) = futures.next().await {
        for log in result? {
            match logs.get(&log.address) {
                None => {
                    logs.insert(log.address, log);
                }
                Some(old) => {
                    let new_log_index = (log.block_number.unwrap().as_u64(), log.log_index.unwrap().as_u64());
                    let old_log_index = (old.block_number.unwrap().as_u64(), old.log_index.unwrap().as_u64());
                    if new_log_index > old_log_index {
                        logs.insert(log.address, log);
                    }
                }
            }
        }
    }

    let mut logs: Vec<Log> = logs.into_iter().map(|(_, log)| log).collect();
    logs.sort_by(|a, b| {
        let a_index = (a.block_number.unwrap().as_u64(), a.log_index.unwrap().as_u64());
        let b_index = (b.block_number.unwrap().as_u64(), b.log_index.unwrap().as_u64());
        a_index.cmp(&b_index)
    });
    Ok(logs)
}
