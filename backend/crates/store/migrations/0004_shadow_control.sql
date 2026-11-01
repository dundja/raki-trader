-- Whether control was `running` at a shadow evaluation. The shadow now evaluates whenever the
-- safety checks pass, also while a pause waits for a manual resume. Earlier rows evaluated only
-- while running (paused ticks were skipped as `not_running`).
alter table shadow_evals add column control_running boolean;
update shadow_evals set control_running = (reason is distinct from 'not_running');
alter table shadow_evals alter column control_running set not null;
