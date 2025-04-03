#[path = "../../pe/tests/common/mod.rs"]
mod common;

// TODO(FK): test provider with pevm/data/blocks
//#[test]
// fn mainnet_blocks_from_disk() {
//     common::for_each_block_from_disk(|block, storage| {
//         // Run several times to try catching a race condition if there is any.
//         // 1000~2000 is a better choice for local testing after major changes.
//         for _ in 0..3 {
//             common::test_execute_alloy(&Ethereum::mainnet(), &storage, block.clone(), true)
//         }
//     });
// }
