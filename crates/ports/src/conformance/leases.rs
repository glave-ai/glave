//! What every store of leases must do, on its own clock: give a lease to one holder at a time,
//! renew and release it only for its holder, let a lapsed one go to the next, keep
//! organizations and what is leased apart, and give a lease that holders race for to one of
//! them. A lease of no time lapses at once, so the suite needs no clock it can move.

use std::time::Duration;

use super::racing::{WRITERS, together};
use super::{Verdict, ensure, thread, uuid_v7};
use crate::leases::{Holder, Lease, Leased, Leases};
use crate::scope::OrgId;

/// Long enough that nothing in the suite outlives it.
const HOUR: Duration = Duration::from_secs(3_600);

fn holder(n: u64) -> Holder {
    Holder::from_uuid(uuid_v7(5_000 + n))
}

fn org(n: u64) -> OrgId {
    OrgId::from_uuid(uuid_v7(6_000 + n))
}

/// Every [`Leases`] must keep each lease to one holder at a time on the store's clock.
pub async fn leases<L: Leases + ?Sized>(leases: &L) -> Verdict {
    holding(leases).await?;
    lapsing(leases).await?;
    apart(leases).await?;
    racing(leases).await
}

/// One holder at a time: another is refused while it holds; its holder takes it again, renews
/// it and lets it go; and a holder that does not hold it changes nothing.
async fn holding<L: Leases + ?Sized>(leases: &L) -> Verdict {
    let (here, on) = (org(1), Leased::Scheduler);
    let (first, second) = (holder(1), holder(2));
    let lease = leases
        .acquire(here, on, first, HOUR)
        .await
        .map_err(|e| e.to_string())?;
    let Some(lease) = lease else {
        return Err("a lease no one holds must be taken".into());
    };
    ensure(
        lease.org == here && lease.on == on && lease.holder == first,
        "a lease must be the one asked for, by its holder",
    )?;
    let taken = leases
        .acquire(here, on, second, HOUR)
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        taken.is_none(),
        "a lease must not be taken while another holds it",
    )?;
    let again = leases
        .acquire(here, on, first, HOUR)
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        again.is_some_and(|again| again.holder == first && again.expires >= lease.expires),
        "a holder must take its own lease again, renewing it",
    )?;
    let renewed = leases
        .renew(&lease, HOUR)
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        renewed.is_some_and(|renewed| renewed.expires >= lease.expires),
        "a holder must renew its lease",
    )?;
    let posing = Lease {
        holder: second,
        ..lease
    };
    ensure(
        leases
            .renew(&posing, HOUR)
            .await
            .map_err(|e| e.to_string())?
            .is_none(),
        "only its holder must renew a lease",
    )?;
    leases.release(&posing).await.map_err(|e| e.to_string())?;
    let taken = leases
        .acquire(here, on, second, HOUR)
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        taken.is_none(),
        "a release by someone else must leave the lease with its holder",
    )?;
    leases.release(&lease).await.map_err(|e| e.to_string())?;
    ensure(
        leases
            .renew(&lease, HOUR)
            .await
            .map_err(|e| e.to_string())?
            .is_none(),
        "a lease let go must not be renewed",
    )?;
    let taken = leases
        .acquire(here, on, second, HOUR)
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        taken.is_some_and(|taken| taken.holder == second),
        "a lease let go must be anyone's at once",
    )
}

/// A lease that lapsed is anyone's, and its old holder can neither renew nor release it once
/// another took it.
async fn lapsing<L: Leases + ?Sized>(leases: &L) -> Verdict {
    let (here, on) = (org(2), Leased::Thread(thread(1)?));
    let (first, second) = (holder(1), holder(2));
    let lapsed = leases
        .acquire(here, on, first, Duration::ZERO)
        .await
        .map_err(|e| e.to_string())?;
    let Some(lapsed) = lapsed else {
        return Err("a lease no one holds must be taken, for no time too".into());
    };
    let taken = leases
        .acquire(here, on, second, HOUR)
        .await
        .map_err(|e| e.to_string())?;
    let Some(taken) = taken else {
        return Err("a lease that lapsed must be anyone's".into());
    };
    ensure(
        leases
            .renew(&lapsed, HOUR)
            .await
            .map_err(|e| e.to_string())?
            .is_none(),
        "a lease another took after it lapsed must not be renewed by its old holder",
    )?;
    leases.release(&lapsed).await.map_err(|e| e.to_string())?;
    let again = leases
        .acquire(here, on, first, HOUR)
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        again.is_none(),
        "an old holder's release must leave the lease with the one who took it",
    )?;
    ensure(
        leases
            .renew(&taken, HOUR)
            .await
            .map_err(|e| e.to_string())?
            .is_some(),
        "the one who took a lapsed lease must renew it",
    )?;
    let own = leases
        .acquire(here, Leased::Scheduler, first, Duration::ZERO)
        .await
        .map_err(|e| e.to_string())?;
    let Some(own) = own else {
        return Err("a lease no one holds must be taken".into());
    };
    ensure(
        leases
            .renew(&own, HOUR)
            .await
            .map_err(|e| e.to_string())?
            .is_some(),
        "a holder must renew its lease after it lapsed while no one else took it",
    )
}

/// Organizations and what is leased are apart: holding one lease says nothing of another.
async fn apart<L: Leases + ?Sized>(leases: &L) -> Verdict {
    let (here, elsewhere) = (org(3), org(4));
    let (first, second) = (holder(1), holder(2));
    let on = Leased::Thread(thread(2)?);
    let held = leases
        .acquire(here, on, first, HOUR)
        .await
        .map_err(|e| e.to_string())?;
    ensure(held.is_some(), "a lease no one holds must be taken")?;
    let there = leases
        .acquire(elsewhere, on, second, HOUR)
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        there.is_some(),
        "the same lease in another organization must be apart",
    )?;
    let other = Leased::Thread(thread(3)?);
    let beside = leases
        .acquire(here, other, second, HOUR)
        .await
        .map_err(|e| e.to_string())?;
    ensure(beside.is_some(), "another thread's lease must be apart")?;
    let scheduler = leases
        .acquire(here, Leased::Scheduler, second, HOUR)
        .await
        .map_err(|e| e.to_string())?;
    ensure(scheduler.is_some(), "the scheduler's lease must be apart")
}

/// Holders racing for one lease, free or lapsed: exactly one gets it.
async fn racing<L: Leases + ?Sized>(leases: &L) -> Verdict {
    let here = org(5);
    for (n, on) in [Leased::Thread(thread(4)?), Leased::Thread(thread(5)?)]
        .into_iter()
        .enumerate()
    {
        if n == 1 {
            let lapsed = leases
                .acquire(here, on, holder(100), Duration::ZERO)
                .await
                .map_err(|e| e.to_string())?;
            ensure(lapsed.is_some(), "a lease no one holds must be taken")?;
        }
        let racers = (0..WRITERS)
            .map(|racer| async move { leases.acquire(here, on, holder(racer), HOUR).await })
            .collect();
        let results = together(racers).await;
        let won = results
            .iter()
            .filter(|result| matches!(result, Ok(Some(_))))
            .count();
        ensure(
            results.iter().all(Result::is_ok),
            "racing for a lease must not fail",
        )?;
        ensure(
            won == 1,
            &format!("exactly one racing holder must get a lease, not {won}"),
        )?;
    }
    Ok(())
}
