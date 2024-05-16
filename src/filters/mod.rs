use crate::amm::AMM;

pub mod address;
pub mod value;

pub fn filter_empty_amms(amms: Vec<AMM>) -> Vec<AMM> {
    let mut cleaned_amms = vec![];

    for amm in amms.into_iter() {
        match amm {
            AMM::UniswapV2Pool(ref uniswap_v2_pool) => {
                if uniswap_v2_pool.token_a.data_is_populated()
                    && !uniswap_v2_pool.token_b.data_is_populated()
                {
                    cleaned_amms.push(amm)
                }
            }
        }
    }

    cleaned_amms
}
