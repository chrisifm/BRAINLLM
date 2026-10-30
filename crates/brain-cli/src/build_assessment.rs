//! A failed payload budget is still a failed build, but does not erase fresh diagnostic evidence.
use crate::BuildReport;
use brain_mcp::Json;

pub(crate) struct Assessment {
    pub report: BuildReport,
    pub budget_error: Option<String>,
}
impl Assessment {
    pub fn into_report(self) -> Result<BuildReport, String> {
        match self.budget_error { Some(error) => Err(error), None => Ok(self.report) }
    }
    pub fn doctor_json(&self) -> Json {
        let mut fields = vec![
            ("ok".into(), Json::Bool(self.budget_error.is_none())),
            ("freshOutput".into(), Json::Bool(true)),
            ("signature".into(), Json::str(&self.report.signature)),
            ("files".into(), Json::Num(self.report.files.len() as f64)),
            ("budget".into(), Json::obj([("ok", Json::Bool(self.budget_error.is_none()))])),
        ];
        if let Some(error) = &self.budget_error {
            fields.push(("error".into(), Json::str(error)));
            fields.push(("report".into(), crate::build_json(&self.report).get("report").cloned().unwrap_or(Json::Arr(vec![]))));
            fields.push(("ledger".into(), Json::Arr(self.report.ledger.clone())));
        }
        Json::Obj(fields)
    }
}
