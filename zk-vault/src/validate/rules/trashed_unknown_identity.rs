use zk_domain::note::RuleId;

pub(crate) const ID: RuleId = RuleId::TrashedUnknownIdentity;

pub(crate) fn description() -> &'static str {
    ID.description()
}
