use crate::handler::MyHandler;
use revm::{
    handler::{
        evm::FrameTr, instructions::EthInstructions, EthFrame, EthPrecompiles, EvmTr,
        FrameInitOrResult, ItemOrResult, Handler,
    },
    inspector::{InspectorEvmTr, JournalExt, InspectCommitEvm, InspectEvm, Inspector, InspectorHandler},
    interpreter::interpreter::EthInterpreter,
    Database,
    context::{
        result::{ExecResultAndState, HaltReason, InvalidTransaction},
        ContextError, ContextSetters, ContextTr, Evm, FrameStack,
    },
    context_interface::{
        result::{EVMError, ExecutionResult},
        JournalTr,
    },
    state::EvmState,
    DatabaseCommit, ExecuteCommitEvm, ExecuteEvm,
};

/// MyEvm variant of the EVM.
///
/// This struct demonstrates how to create a custom EVM implementation by wrapping
/// the standard REVM components. It combines a context (CTX), an inspector (INSP),
/// and the standard Ethereum instructions, precompiles, and frame execution logic.
///
/// The generic parameters allow for flexibility in the underlying database and
/// inspection capabilities while maintaining the standard Ethereum execution semantics.
#[derive(Debug)]
pub struct MyEvm<CTX, INSP>(
    pub  Evm<
        CTX,
        INSP,
        EthInstructions<EthInterpreter, CTX>,
        EthPrecompiles,
        EthFrame<EthInterpreter>,
    >,
);

impl<CTX: ContextTr, INSP> MyEvm<CTX, INSP> {
    pub fn new(ctx: CTX, inspector: INSP) -> Self {
        Self(Evm {
            ctx,
            inspector,
            instruction: EthInstructions::new_mainnet(),
            precompiles: EthPrecompiles::default(),
            frame_stack: FrameStack::new(),
        })
    }
}

impl<CTX: ContextTr, INSP> EvmTr for MyEvm<CTX, INSP>
where
    CTX: ContextTr,
{
    type Context = CTX;
    type Instructions = EthInstructions<EthInterpreter, CTX>;
    type Precompiles = EthPrecompiles;
    type Frame = EthFrame<EthInterpreter>;

    #[inline]
    fn frame_init(
        &mut self,
        frame_input: <Self::Frame as FrameTr>::FrameInit,
    ) -> Result<
        ItemOrResult<&mut Self::Frame, <Self::Frame as FrameTr>::FrameResult>,
        ContextError<<<Self::Context as ContextTr>::Db as Database>::Error>,
    > {
        self.0.frame_init(frame_input)
    }

    #[inline]
    fn frame_run(
        &mut self,
    ) -> Result<
        FrameInitOrResult<Self::Frame>,
        ContextError<<<Self::Context as ContextTr>::Db as Database>::Error>,
    > {
        self.0.frame_run()
    }

    #[inline]
    fn frame_return_result(
        &mut self,
        frame_result: <Self::Frame as FrameTr>::FrameResult,
    ) -> Result<
        Option<<Self::Frame as FrameTr>::FrameResult>,
        ContextError<<<Self::Context as ContextTr>::Db as Database>::Error>,
    > {
        self.0.frame_return_result(frame_result)
    }
    
    #[doc = " Returns a mutable reference to the execution context"]
    fn ctx(&mut self) ->  &mut Self::Context {
        self.0.ctx()
    }
    
    #[doc = " Returns an immutable reference to the execution context"]
    fn ctx_ref(&self) ->  &Self::Context {
        self.0.ctx_ref()
    }
    
    #[doc = " Returns mutable references to both the context and instruction set."]
    #[doc = " This enables atomic access to both components when needed."]
    fn ctx_instructions(&mut self) -> (&mut Self::Context, &mut Self::Instructions) {
        self.0.ctx_instructions()
    }
    
    #[doc = " Returns mutable references to both the context and precompiles."]
    #[doc = " This enables atomic access to both components when needed."]
    fn ctx_precompiles(&mut self) -> (&mut Self::Context, &mut Self::Precompiles) {
        self.0.ctx_precompiles()
    }
    
    #[doc = " Returns a mutable reference to the frame stack."]
    fn frame_stack(&mut self) ->  &mut FrameStack<Self::Frame>  {
        self.0.frame_stack()
    }
}

impl<CTX: ContextTr, INSP> InspectorEvmTr for MyEvm<CTX, INSP>
where
    CTX: ContextSetters<Journal: JournalExt>,
    INSP: Inspector<CTX, EthInterpreter>,
{
    type Inspector = INSP;
    
    fn inspector(&mut self) -> &mut Self::Inspector {
        self.0.inspector()
    }
    
    fn ctx_inspector(&mut self) -> (&mut Self::Context, &mut Self::Inspector) {
        self.0.ctx_inspector()
    }
    
    fn ctx_inspector_frame(
        &mut self,
    ) -> (&mut Self::Context, &mut Self::Inspector, &mut Self::Frame) {
        self.0.ctx_inspector_frame()
    }
    
    fn ctx_inspector_frame_instructions(
        &mut self,
    ) -> (
        &mut Self::Context,
        &mut Self::Inspector,
        &mut Self::Frame,
        &mut Self::Instructions,
    ) {
        self.0.ctx_inspector_frame_instructions()
    }
}

/// Type alias for the error type of the OpEvm.
type MyError<CTX> = EVMError<<<CTX as ContextTr>::Db as Database>::Error, InvalidTransaction>;

// Trait that allows to replay and transact the transaction.
impl<CTX, INSP> ExecuteEvm for MyEvm<CTX, INSP>
where
    CTX: ContextSetters<Journal: JournalTr<State = EvmState>>,
{
    type State = EvmState;
    type ExecutionResult = ExecutionResult<HaltReason>;
    type Error = MyError<CTX>;

    type Tx = <CTX as ContextTr>::Tx;

    type Block = <CTX as ContextTr>::Block;

    fn set_block(&mut self, block: Self::Block) {
        self.0.ctx.set_block(block);
    }

    fn transact_one(&mut self, tx: Self::Tx) -> Result<Self::ExecutionResult, Self::Error> {
        self.0.ctx.set_tx(tx);
        let mut handler = MyHandler::default();
        handler.run(self)
    }

    fn finalize(&mut self) -> Self::State {
        self.ctx().journal_mut().finalize()
    }

    fn replay(
        &mut self,
    ) -> Result<ExecResultAndState<Self::ExecutionResult, Self::State>, Self::Error> {
        let mut handler = MyHandler::default();
        handler.run(self).map(|result| {
            let state = self.finalize();
            ExecResultAndState::new(result, state)
        })
    }
}

// Trait allows replay_commit and transact_commit functionality.
impl<CTX, INSP> ExecuteCommitEvm for MyEvm<CTX, INSP>
where
    CTX: ContextSetters<Db: DatabaseCommit, Journal: JournalTr<State = EvmState>>,
{
    fn commit(&mut self, state: Self::State) {
        self.ctx().db_mut().commit(state);
    }
}

// Inspection trait.
impl<CTX, INSP> InspectEvm for MyEvm<CTX, INSP>
where
    CTX: ContextSetters<Journal: JournalTr<State = EvmState> + JournalExt>,
    INSP: Inspector<CTX, EthInterpreter>,
{
    type Inspector = INSP;

    fn set_inspector(&mut self, inspector: Self::Inspector) {
        self.0.inspector = inspector;
    }

    fn inspect_one_tx(&mut self, tx: Self::Tx) -> Result<Self::ExecutionResult, Self::Error> {
        self.0.ctx.set_tx(tx);
        let mut handler = MyHandler::default();
        handler.inspect_run(self)
    }
}

// Inspect
impl<CTX, INSP> InspectCommitEvm for MyEvm<CTX, INSP>
where
    CTX: ContextSetters<Db: DatabaseCommit, Journal: JournalTr<State = EvmState> + JournalExt>,
    INSP: Inspector<CTX, EthInterpreter>,
{
}