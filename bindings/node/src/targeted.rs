use crate::error::{catch_panic, napi_error};
use cellrune_binding_support::SharedWorkbookSession;
use cellrune_interop::{CancellationToken, TargetCalculationRequestDto};
use napi::{Env, Task};
use std::sync::Arc;

pub struct TargetCalculationTask {
    pub(crate) session: Arc<SharedWorkbookSession>,
    pub(crate) request: TargetCalculationRequestDto,
    pub(crate) cancellation: CancellationToken,
}

impl Task for TargetCalculationTask {
    type Output = String;
    type JsValue = String;
    fn compute(&mut self) -> napi::Result<Self::Output> {
        catch_panic(|| {
            let response = cellrune_binding_support::calculate_targets_cancellable(
                &self.session,
                &self.request,
                self.cancellation.clone(),
            )
            .map_err(napi_error)?;
            crate::preview_json::serialize(&response)
        })
    }
    fn resolve(&mut self, _env: Env, output: Self::Output) -> napi::Result<Self::JsValue> {
        Ok(output)
    }
}
