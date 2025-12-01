use alloy_evm::Evm as EvmTrait;
use alloy_evm::{env::EvmEnv, eth::EthEvmContext};
use alloy_primitives::{Address, Bytes};
use revm::{
    Database, ExecuteEvm, SystemCallEvm,
    context::{BlockEnv, Context, Evm as RevmEvm, FrameStack, TxEnv, result::HaltReason},
    context_interface::result::{EVMError, ResultAndState},
    handler::{
        EthFrame, EthPrecompiles, EvmTr, FrameInitOrResult, PrecompileProvider,
        evm::{ContextDbError, FrameInitResult, FrameTr},
        instructions::EthInstructions,
    },
    inspector::{InspectEvm, Inspector, InspectorEvmTr},
    interpreter::{InterpreterResult, interpreter::EthInterpreter},
    primitives::hardfork::SpecId,
};
use std::ops::{Deref, DerefMut};

#[derive(Debug)]
pub struct MyEvm<DB: Database, I, PRECOMPILE = EthPrecompiles> {
    pub inner: RevmEvm<
        EthEvmContext<DB>,
        I,
        EthInstructions<EthInterpreter, EthEvmContext<DB>>,
        PRECOMPILE,
        EthFrame,
    >,
    inspect: bool,
}

impl<DB: Database, I, PRECOMPILE> MyEvm<DB, I, PRECOMPILE> {
    /// Creates a new Ethereum EVM instance.
    ///
    /// The `inspect` argument determines whether the configured [`Inspector`] of the given
    /// [`RevmEvm`] should be invoked on [`Evm::transact`].
    pub const fn new(
        evm: RevmEvm<
            EthEvmContext<DB>,
            I,
            EthInstructions<EthInterpreter, EthEvmContext<DB>>,
            PRECOMPILE,
            EthFrame,
        >,
        inspect: bool,
    ) -> Self {
        Self {
            inner: evm,
            inspect,
        }
    }

    /// Consumes self and return the inner EVM instance.
    pub fn into_inner(
        self,
    ) -> RevmEvm<
        EthEvmContext<DB>,
        I,
        EthInstructions<EthInterpreter, EthEvmContext<DB>>,
        PRECOMPILE,
        EthFrame,
    > {
        self.inner
    }

    /// Provides a reference to the EVM context.
    pub const fn ctx(&self) -> &EthEvmContext<DB> {
        &self.inner.ctx
    }

    /// Provides a mutable reference to the EVM context.
    pub const fn ctx_mut(&mut self) -> &mut EthEvmContext<DB> {
        &mut self.inner.ctx
    }
}

impl<DB: Database, I, PRECOMPILE> Deref for MyEvm<DB, I, PRECOMPILE> {
    type Target = EthEvmContext<DB>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.ctx()
    }
}

impl<DB: Database, I, PRECOMPILE> DerefMut for MyEvm<DB, I, PRECOMPILE> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.ctx_mut()
    }
}

impl<DB, I, PRECOMPILE> EvmTrait for MyEvm<DB, I, PRECOMPILE>
where
    DB: alloy_evm::Database,
    I: Inspector<EthEvmContext<DB>>,
    PRECOMPILE: PrecompileProvider<EthEvmContext<DB>, Output = InterpreterResult>,
{
    type DB = DB;
    type Tx = TxEnv;
    type BlockEnv = BlockEnv;
    type Error = EVMError<DB::Error>;
    type HaltReason = HaltReason;
    type Spec = SpecId;
    type Precompiles = PRECOMPILE;
    type Inspector = I;

    fn block(&self) -> &Self::BlockEnv {
        &self.inner.ctx.block
    }

    fn chain_id(&self) -> u64 {
        self.inner.ctx.cfg.chain_id
    }

    fn transact_raw(
        &mut self,
        tx: Self::Tx,
    ) -> Result<ResultAndState<Self::HaltReason>, Self::Error> {
        if self.inspect {
            self.inner.inspect_tx(tx)
        } else {
            ExecuteEvm::transact(self, tx)
        }
    }

    fn transact_system_call(
        &mut self,
        caller: Address,
        contract: Address,
        data: Bytes,
    ) -> Result<ResultAndState<Self::HaltReason>, Self::Error> {
        // self.inner.system_call_with_caller(caller, contract, data)
        self.inner.system_call_with_caller(caller, contract, data)
    }

    fn finish(self) -> (Self::DB, EvmEnv<Self::Spec>) {
        let Context {
            block: block_env,
            cfg: cfg_env,
            journaled_state,
            ..
        } = self.inner.ctx;

        (journaled_state.database, EvmEnv { block_env, cfg_env })
    }

    fn set_inspector_enabled(&mut self, enabled: bool) {
        self.inspect = enabled;
    }

    fn components(&self) -> (&Self::DB, &Self::Inspector, &Self::Precompiles) {
        (
            &self.inner.ctx.journaled_state.database,
            &self.inner.inspector,
            &self.inner.precompiles,
        )
    }

    fn components_mut(&mut self) -> (&mut Self::DB, &mut Self::Inspector, &mut Self::Precompiles) {
        (
            &mut self.inner.ctx.journaled_state.database,
            &mut self.inner.inspector,
            &mut self.inner.precompiles,
        )
    }
}

impl<DB, I, PRECOMPILE> EvmTr for MyEvm<DB, I, PRECOMPILE>
where
    DB: alloy_evm::Database,
    I: Inspector<EthEvmContext<DB>>,
    PRECOMPILE: PrecompileProvider<EthEvmContext<DB>, Output = InterpreterResult>,
{
    type Context = EthEvmContext<DB>;
    type Instructions = EthInstructions<EthInterpreter, EthEvmContext<DB>>;
    type Precompiles = PRECOMPILE;
    type Frame = EthFrame<EthInterpreter>;

    fn ctx(&mut self) -> &mut Self::Context {
        self.inner.ctx_mut()
    }

    fn ctx_ref(&self) -> &Self::Context {
        self.inner.ctx_ref()
    }

    fn ctx_instructions(&mut self) -> (&mut Self::Context, &mut Self::Instructions) {
        self.inner.ctx_instructions()
    }

    fn ctx_precompiles(&mut self) -> (&mut Self::Context, &mut Self::Precompiles) {
        self.inner.ctx_precompiles()
    }

    fn frame_stack(&mut self) -> &mut FrameStack<Self::Frame> {
        self.inner.frame_stack()
    }

    fn frame_init(
        &mut self,
        frame_input: <Self::Frame as FrameTr>::FrameInit,
    ) -> Result<FrameInitResult<'_, Self::Frame>, ContextDbError<Self::Context>> {
        self.inner.frame_init(frame_input)
    }

    fn frame_run(
        &mut self,
    ) -> Result<FrameInitOrResult<Self::Frame>, ContextDbError<Self::Context>> {
        self.inner.frame_run()
    }

    fn frame_return_result(
        &mut self,
        result: <Self::Frame as FrameTr>::FrameResult,
    ) -> Result<Option<<Self::Frame as FrameTr>::FrameResult>, ContextDbError<Self::Context>> {
        self.inner.frame_return_result(result)
    }

    fn all(
        &self,
    ) -> (
        &Self::Context,
        &Self::Instructions,
        &Self::Precompiles,
        &FrameStack<Self::Frame>,
    ) {
        (
            self.inner.ctx_ref(),
            &self.inner.instruction,
            &self.inner.precompiles,
            &self.inner.frame_stack,
        )
    }

    fn all_mut(
        &mut self,
    ) -> (
        &mut Self::Context,
        &mut Self::Instructions,
        &mut Self::Precompiles,
        &mut FrameStack<Self::Frame>,
    ) {
        (
            &mut self.inner.ctx,
            &mut self.inner.instruction,
            &mut self.inner.precompiles,
            &mut self.inner.frame_stack,
        )
    }
}

impl<DB, I, PRECOMPILE> InspectorEvmTr for MyEvm<DB, I, PRECOMPILE>
where
    DB: alloy_evm::Database,
    I: Inspector<EthEvmContext<DB>>,
    PRECOMPILE: PrecompileProvider<EthEvmContext<DB>, Output = InterpreterResult>,
{
    type Inspector = I;

    fn inspector(&mut self) -> &mut Self::Inspector {
        self.inner.inspector()
    }

    fn ctx_inspector(&mut self) -> (&mut Self::Context, &mut Self::Inspector) {
        self.inner.ctx_inspector()
    }

    fn ctx_inspector_frame(
        &mut self,
    ) -> (&mut Self::Context, &mut Self::Inspector, &mut Self::Frame) {
        self.inner.ctx_inspector_frame()
    }

    fn ctx_inspector_frame_instructions(
        &mut self,
    ) -> (
        &mut Self::Context,
        &mut Self::Inspector,
        &mut Self::Frame,
        &mut Self::Instructions,
    ) {
        self.inner.ctx_inspector_frame_instructions()
    }

    fn all_inspector(
        &self,
    ) -> (
        &Self::Context,
        &Self::Instructions,
        &Self::Precompiles,
        &FrameStack<Self::Frame>,
        &Self::Inspector,
    ) {
        (
            self.inner.ctx_ref(),
            &self.inner.instruction,
            &self.inner.precompiles,
            &self.inner.frame_stack,
            &self.inner.inspector,
        )
    }

    fn all_mut_inspector(
        &mut self,
    ) -> (
        &mut Self::Context,
        &mut Self::Instructions,
        &mut Self::Precompiles,
        &mut FrameStack<Self::Frame>,
        &mut Self::Inspector,
    ) {
        (
            &mut self.inner.ctx,
            &mut self.inner.instruction,
            &mut self.inner.precompiles,
            &mut self.inner.frame_stack,
            &mut self.inner.inspector,
        )
    }
}
