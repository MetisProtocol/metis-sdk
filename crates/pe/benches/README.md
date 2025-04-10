# Benchmarks

## Gigagas

Run the benchmark:

```shell
JEMALLOC_SYS_WITH_MALLOC_CONF="thp:always,metadata_thp:always" cargo bench --features global-alloc --bench gigagas
```

Run the benchmark with the compiler feature

```shell
JEMALLOC_SYS_WITH_MALLOC_CONF="thp:always,metadata_thp:always" cargo bench --features global-alloc --bench gigagas --features compiler
```

|                 | No. Transactions | Gas Used      | Sequential (ms) | Parallel (ms) | Speedup    |
| --------------- | ---------------- | ------------- | --------------- | ------------- | ---------- |
| Raw Transfers   | 47,620           | 1,000,020,000 | 159.08          | 56.425        | 🟢2.82     |
| ERC20 Transfers | 37,123           | 1,000,019,374 | 246.43          | 60.817        | 🟢4.05     |
| Uniswap Swaps   | 6,413            | 1,000,004,742 | 413.42          | 18.707        | 🟢**22.1** |
