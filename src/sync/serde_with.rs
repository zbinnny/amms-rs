use std::collections::HashMap;

use ethers::prelude::H160;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::amm::factory::{AutomatedMarketMakerFactory, Factory};
use crate::amm::{AutomatedMarketMaker, AMM};
use crate::currency::Currency;

pub trait H160Map {
    fn key(&self) -> H160;
}

impl H160Map for Currency {
    fn key(&self) -> H160 {
        self.address()
    }
}

impl H160Map for AMM {
    fn key(&self) -> H160 {
        self.address()
    }
}

impl H160Map for Factory {
    fn key(&self) -> H160 {
        self.address()
    }
}

// 自定义的通用的反序列化函数，将 Vec<T> 转换为 HashMap<K, T>
pub fn deserialize_vec_to_map<'de, D, T>(deserializer: D) -> Result<HashMap<H160, T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + H160Map,
{
    let vec: Vec<T> = Deserialize::deserialize(deserializer)?;

    Ok(vec.into_iter().map(|item| (item.key(), item)).collect())
}

// 自定义的通用的序列化函数，将 HashMap<K, T> 转换为 Vec<T>
pub fn serialize_map_to_vec<S, T>(map: &HashMap<H160, T>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    T: Serialize + Clone,
{
    let vec: Vec<T> = map.values().cloned().collect();
    vec.serialize(serializer)
}
