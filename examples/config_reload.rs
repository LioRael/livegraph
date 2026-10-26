//! In-memory configuration reload with rollback on failure.

use livegraph::{GraphBuilder, Result};

#[derive(Debug)]
struct Routes {
    upstream: String,
    max_connections: u32,
}

fn main() -> Result<()> {
    let mut b = GraphBuilder::new();
    let host = b.resource("upstream host", String::from("localhost"));
    let limit = b.resource("connection limit text", String::from("100"));
    let parsed = b.derived("parsed limit", [limit.dep()], move |cx| {
        cx.get(limit)?
            .parse::<u32>()
            .map_err(|e| e.to_string().into())
    })?;
    let routes = b.derived("routes", [host.dep(), parsed.dep()], move |cx| {
        Ok(Routes {
            upstream: format!("https://{}", cx.get(host)?),
            max_connections: *cx.get(parsed)?,
        })
    })?;
    b.validate("connection budget", move |s| {
        if s.get(routes)?.max_connections > 1000 {
            Err("connection limit exceeds 1000".into())
        } else {
            Ok(())
        }
    });
    let graph = b.build()?;
    let pinned = graph.snapshot();
    let mut changes = graph.changes();
    changes
        .set(host, "api.example.com".into())?
        .set(limit, "250".into())?;
    let updated = graph.commit(changes)?;
    println!("old v{}: {:?}", pinned.version(), pinned.get(routes)?);
    println!("new v{}: {:?}", updated.version(), updated.get(routes)?);
    assert_eq!(pinned.get(routes)?.upstream, "https://localhost");
    assert_eq!(updated.get(routes)?.upstream, "https://api.example.com");
    for invalid in ["not a number", "5000"] {
        let mut changes = graph.changes();
        changes.set(limit, invalid.into())?;
        let error = graph.commit(changes).expect_err("invalid update accepted");
        println!("rejected: {error}");
        assert_eq!(graph.snapshot().version(), 1);
    }
    graph.with_snapshot(|snapshot| -> Result<()> {
        println!(
            "still serving v{}: {:?}",
            snapshot.version(),
            snapshot.get(routes)?
        );
        Ok(())
    })?;
    Ok(())
}
