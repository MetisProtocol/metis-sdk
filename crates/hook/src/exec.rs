use crate::evm::MyEvm;
use crate::handler::MyHandler;
use alloy_evm::eth::EthEvmContext;
use reth_evm::Database;
use revm::{
    DatabaseCommit, ExecuteCommitEvm, ExecuteEvm,
    context::{BlockEnv, ContextSetters, TxEnv, result::ExecResultAndState},
    context_interface::{
        ContextTr,
        result::{EVMError, ExecutionResult},
    },
    handler::{Handler, PrecompileProvider},
    inspector::{InspectCommitEvm, InspectEvm, Inspector, InspectorHandler},
    interpreter::InterpreterResult,
    state::EvmState,
};

impl<DB, I, PRECOMPILE> ExecuteEvm for MyEvm<DB, I, PRECOMPILE>
where
    DB: Database,
    I: Inspector<EthEvmContext<DB>>,
    PRECOMPILE: PrecompileProvider<EthEvmContext<DB>, Output = InterpreterResult>,
{
    type ExecutionResult = ExecutionResult;
    type State = EvmState;
    type Error = EVMError<DB::Error>;
    type Tx = TxEnv;
    type Block = BlockEnv;

    fn set_block(&mut self, block: Self::Block) {
        self.inner.set_block(block);
    }

    fn transact_one(&mut self, tx: Self::Tx) -> Result<Self::ExecutionResult, Self::Error> {
        // self.inner.transact_one(tx)
        self.inner.set_tx(tx);
        let mut handler = MyHandler::default();
        handler.run(self)
    }

    fn finalize(&mut self) -> Self::State {
        self.inner.finalize()
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

impl<DB, I, PRECOMPILE> ExecuteCommitEvm for MyEvm<DB, I, PRECOMPILE>
where
    DB: Database + DatabaseCommit,
    I: Inspector<EthEvmContext<DB>>,
    PRECOMPILE: PrecompileProvider<EthEvmContext<DB>, Output = InterpreterResult>,
{
    fn commit(&mut self, state: Self::State) {
        self.inner.ctx.db_mut().commit(state);
    }
}

impl<DB, I, PRECOMPILE> InspectEvm for MyEvm<DB, I, PRECOMPILE>
where
    DB: Database,
    I: Inspector<EthEvmContext<DB>>,
    PRECOMPILE: PrecompileProvider<EthEvmContext<DB>, Output = InterpreterResult>,
{
    type Inspector = I;

    fn set_inspector(&mut self, inspector: Self::Inspector) {
        self.inner.set_inspector(inspector);
    }

    fn inspect_one_tx(&mut self, tx: Self::Tx) -> Result<Self::ExecutionResult, Self::Error> {
        self.inner.set_tx(tx);
        let mut handler = MyHandler::default();
        handler.inspect_run(self)
    }
}

impl<DB, I, PRECOMPILE> InspectCommitEvm for MyEvm<DB, I, PRECOMPILE>
where
    DB: Database + DatabaseCommit,
    I: Inspector<EthEvmContext<DB>>,
    PRECOMPILE: PrecompileProvider<EthEvmContext<DB>, Output = InterpreterResult>,
{
}
