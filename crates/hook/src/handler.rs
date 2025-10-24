use revm::{
    context::result::{EVMError, HaltReason, InvalidTransaction},
    context_interface::{ContextTr, JournalTr},
    handler::{
        evm::FrameTr, instructions::InstructionProvider, EvmTr, FrameResult, Handler,
        PrecompileProvider,
    },
    inspector::{Inspector, InspectorEvmTr, InspectorHandler},
    interpreter::{interpreter::EthInterpreter, interpreter_action::FrameInit, InterpreterResult},
    state::EvmState,
    Database,
};
use metis_primitives::{Block, Transaction, U256};

/// Custom handler for MyEvm that defines transaction execution behavior.
///
/// This handler demonstrates how to customize EVM execution by implementing
/// the Handler trait. It can be extended to add custom validation, modify
/// gas calculations, or implement protocol-specific behavior while maintaining
/// compatibility with the standard EVM execution flow.
#[derive(Debug)]
pub struct MyHandler<EVM> {
    /// Phantom data to maintain the EVM type parameter.
    /// This field exists solely to satisfy Rust's type system requirements
    /// for generic parameters that aren't directly used in the struct fields.
    pub _phantom: core::marker::PhantomData<EVM>,
}

impl<EVM> Default for MyHandler<EVM> {
    fn default() -> Self {
        Self {
            _phantom: core::marker::PhantomData,
        }
    }
}

impl<EVM> Handler for MyHandler<EVM>
where
    EVM: EvmTr<
        Context: ContextTr<Journal: JournalTr<State = EvmState>>,
        Precompiles: PrecompileProvider<EVM::Context, Output = InterpreterResult>,
        Instructions: InstructionProvider<
            Context = EVM::Context,
            InterpreterTypes = EthInterpreter,
        >,
        Frame: FrameTr<FrameResult = FrameResult, FrameInit = FrameInit>,
    >,
{
    type Evm = EVM;
    type Error = EVMError<<<EVM::Context as ContextTr>::Db as Database>::Error, InvalidTransaction>;
    type HaltReason = HaltReason;

    fn reward_beneficiary(
        &self,
        evm: &mut Self::Evm,
        exec_result: &mut FrameResult,
    ) -> Result<(), Self::Error> {
        let context = evm.ctx();
        let gas = exec_result.gas();
        let beneficiary = context.block().beneficiary();
        // let basefee = context.block().basefee() as u128;
        let coinbase_gas_price = context.tx().effective_gas_price(0);

        // Transfer fee to coinbase/beneficiary.
        // EIP-1559 discard basefee for coinbase transfer. Basefee amount of gas is discarded.
        // let coinbase_gas_price = if context.cfg().spec().into().is_enabled_in(SpecId::LONDON) {
        //     effective_gas_price.saturating_sub(basefee)
        // } else {
        //     effective_gas_price
        // };

        // reward beneficiary
        context.journal_mut().balance_incr(
            beneficiary,
            U256::from(coinbase_gas_price * gas.used() as u128),
        )?;

        Ok(())
    }
}

impl<EVM> InspectorHandler for MyHandler<EVM>
where
    EVM: InspectorEvmTr<
        Inspector: Inspector<<<Self as Handler>::Evm as EvmTr>::Context, EthInterpreter>,
        Context: ContextTr<Journal: JournalTr<State = EvmState>>,
        Precompiles: PrecompileProvider<EVM::Context, Output = InterpreterResult>,
        Instructions: InstructionProvider<
            Context = EVM::Context,
            InterpreterTypes = EthInterpreter,
        >,
    >,
{
    type IT = EthInterpreter;
}