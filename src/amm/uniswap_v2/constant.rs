use ruint::Uint;

pub const U256_0XFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF: Uint<256, 4> =
    Uint::<256, 4>::from_limbs([
        18446744073709551615,
        18446744073709551615,
        18446744073709551615,
        0,
    ]);

pub const U256_0XFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF: Uint<256, 4> =
    Uint::<256, 4>::from_limbs([18446744073709551615, 18446744073709551615, 0, 0]);

pub const U256_0X100000000: Uint<256, 4> = Uint::<256, 4>::from_limbs([4294967296, 0, 0, 0]);
pub const U256_0X10000: Uint<256, 4> = Uint::<256, 4>::from_limbs([65536, 0, 0, 0]);
pub const U256_0X100: Uint<256, 4> = Uint::<256, 4>::from_limbs([256, 0, 0, 0]);
pub const U256_255: Uint<256, 4> = Uint::<256, 4>::from_limbs([255, 0, 0, 0]);
pub const U256_192: Uint<256, 4> = Uint::<256, 4>::from_limbs([192, 0, 0, 0]);
pub const U256_191: Uint<256, 4> = Uint::<256, 4>::from_limbs([191, 0, 0, 0]);
pub const U256_128: Uint<256, 4> = Uint::<256, 4>::from_limbs([128, 0, 0, 0]);
pub const U256_64: Uint<256, 4> = Uint::<256, 4>::from_limbs([64, 0, 0, 0]);
pub const U256_32: Uint<256, 4> = Uint::<256, 4>::from_limbs([32, 0, 0, 0]);
pub const U256_16: Uint<256, 4> = Uint::<256, 4>::from_limbs([16, 0, 0, 0]);
pub const U256_8: Uint<256, 4> = Uint::<256, 4>::from_limbs([8, 0, 0, 0]);
pub const U256_4: Uint<256, 4> = Uint::<256, 4>::from_limbs([4, 0, 0, 0]);
pub const U256_2: Uint<256, 4> = Uint::<256, 4>::from_limbs([2, 0, 0, 0]);
pub const U256_1: Uint<256, 4> = Uint::<256, 4>::from_limbs([1, 0, 0, 0]);
