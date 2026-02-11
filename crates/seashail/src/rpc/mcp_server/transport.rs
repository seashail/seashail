use serde::Serialize;

pub async fn write_frame<W, T>(out: &mut W, v: &T) -> eyre::Result<()>
where
    W: tokio::io::AsyncWrite + Unpin + Send,
    T: Serialize + Sync,
{
    use tokio::io::AsyncWriteExt as _;

    out.write_all(format!("{}\n", serde_json::to_string(v)?).as_bytes())
        .await?;
    out.flush().await?;
    Ok(())
}
