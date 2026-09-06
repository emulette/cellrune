use crate::error::napi_error;
use cellrune_binding_support::SharedWorkbookSession;
use cellrune_interop::TargetCalculationRequestDto;
use napi::{Env, Task};
use std::sync::Arc;

pub struct TargetCalculationTask {
    pub(crate) session: Arc<SharedWorkbookSession>,
    pub(crate) request: TargetCalculationRequestDto,
}

impl Task for TargetCalculationTask {
    type Output = String;
    type JsValue = String;
    fn compute(&mut self) -> napi::Result<Self::Output> {
        let response = cellrune_binding_support::calculate_targets(&self.session, &self.request)
            .map_err(napi_error)?;
        crate::preview_json::serialize(&response)
    }
    fn resolve(&mut self, _env: Env, output: Self::Output) -> napi::Result<Self::JsValue> {
        Ok(output)
    }
}
