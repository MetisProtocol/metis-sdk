# Gigagas Benchmark

Run the benchmark:

```shell
cargo bench --bench gigagas
```

Run the benchmark with the jemalloc feature:

```shell
JEMALLOC_SYS_WITH_MALLOC_CONF="thp:always,metadata_thp:always" cargo bench --features jemalloc --bench gigagas
```

Run the benchmark with the jemalloc and compiler feature:

```shell
JEMALLOC_SYS_WITH_MALLOC_CONF="thp:always,metadata_thp:always" cargo bench --features jemalloc --features compiler --bench gigagas 
```
