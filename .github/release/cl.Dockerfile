# Use Rust Nightly version base image
#FROM rustlang/rust:nightly
FROM --platform=$BUILDPLATFORM rustlang/rust:nightly AS builder

# Set working directory
WORKDIR /app

# Install necessary dependencies, including protoc
RUN apt-get update && apt-get install -y \
    protobuf-compiler \
    gcc-aarch64-linux-gnu \
    libc6-dev-arm64-cross \
    && rm -rf /var/lib/apt/lists/*

# Clone project code
RUN git clone -b dev-096 https://github.com/MetisProtocol/malachite.git
RUN git clone -b main https://github.com/MetisProtocol/malaketh-layered.git

WORKDIR /app/malaketh-layered

RUN rustup target add aarch64-unknown-linux-gnu

ARG TARGETARCH

# Build the project
RUN if [ "$TARGETARCH" = "arm64" ]; then \
        export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc; \
        cargo build --release --target aarch64-unknown-linux-gnu; \
    else \
        cargo build --release; \
    fi

# Copy the generated binary to a new lightweight image
FROM ubuntu:24.04

# Install necessary dependencies
RUN apt-get update && apt-get install -y \
    libssl-dev \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Set working directory
WORKDIR /app

ARG TARGETARCH

RUN if [ "$TARGETARCH" = "arm64" ]; then \
        echo "aarch64-unknown-linux-gnu/" > /target-subdir.txt; \
    else \
        echo "" > /target-subdir.txt; \
    fi

RUN --mount=from=builder,source=/app/malaketh-layered/target,target=/builder-target \
    TARGET_SUBDIR=$(cat /target-subdir.txt) && \
    cp /builder-target/${TARGET_SUBDIR}release/malachitebft-eth-app /app/


# Set environment variables
ENV PATH="/app:${PATH}"

# Expose necessary ports
EXPOSE 8545 8551 9090 3000

# Run the application (assuming you want to run malachitebft-eth-app here)
CMD ["malachitebft-eth-app"]
