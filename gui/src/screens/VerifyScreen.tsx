import { AnimatePresence, motion } from "motion/react";
import { DrivePicker } from "../components/DrivePicker";
import { ErrorAlert, JobActivity } from "../components/Shell";
import { Badge, Button, Field, Hint, Panel, ProgressBar, TextInput } from "../components/ui";
import { dialogs } from "../lib/api";
import { fmtBytes, percent } from "../lib/format";
import { startHash, stopHash } from "../store/actions";
import { useApp } from "../store/app";
import { type HashResult, useVerify } from "../store/verify";

export function VerifyScreen() {
  const source = useVerify((s) => s.source);
  const running = useApp((s) => s.running);
  const size = useVerify((s) => s.size);
  const done = useVerify((s) => s.done);
  const setSource = (v: string) => useVerify.setState({ source: v });

  return (
    <>
      <h2>Verify an image</h2>
      <p className="lead">
        Computes the SHA-256 fingerprint of every byte. Hash the image and its copy (or the drive and its image): the same fingerprint means
        they are identical.
      </p>
      <ErrorAlert />
      <div className="cols">
        <div>
          <DrivePicker selected={source} onPick={setSource} />
          <Panel label="Hash settings">
            <Field
              id="hsrc"
              label="Source"
              action={
                <Button
                  onClick={async () => {
                    const v = await dialogs.openFile("Choose an image file");
                    if (v !== null) setSource(v);
                  }}
                >
                  Browse
                </Button>
              }
            >
              <TextInput id="hsrc" value={source} placeholder="E:\rescue\disk.img or \\.\PhysicalDrive1" onChange={(e) => setSource(e.target.value)} />
            </Field>
            <Hint>
              Reads the whole source once, start to end, so it takes as long as copying it. A drive with unreadable sectors has no single hash:
              image it first and hash the image.
            </Hint>
            <div className="actions">
              <Button variant="primary" disabled={!!running} onClick={() => void startHash(source)}>
                Compute SHA-256
              </Button>
              <Button disabled={running !== "hash"} onClick={stopHash}>
                Stop
              </Button>
            </div>
            {running === "hash" && (
              <div className="spaced">
                <ProgressBar value={size ? percent(done, size) : null} label="Hash progress" />
                <JobActivity job="hash" />
              </div>
            )}
          </Panel>
        </div>
        <div>
          <Results />
        </div>
      </div>
    </>
  );
}

function Results() {
  const results = useVerify((s) => s.results);
  return (
    <Panel title="Fingerprints this session">
      {results.length === 0 ? (
        <p className="dim">No hashes yet. Each one you compute is listed here, newest first, and matching ones are marked.</p>
      ) : (
        <ul className="hashes">
          <AnimatePresence initial={false}>
            {results.map((r) => (
              <motion.li
                key={`${r.at.getTime()}-${r.source}`}
                layout
                initial={{ opacity: 0, y: -8 }}
                animate={{ opacity: 1, y: 0 }}
                transition={{ type: "spring", bounce: 0, visualDuration: 0.3 }}
              >
                <Row r={r} all={results} />
              </motion.li>
            ))}
          </AnimatePresence>
        </ul>
      )}
    </Panel>
  );
}

const baseName = (p: string) => p.split(/[\\/]/).pop() || p;

function Row({ r, all }: { r: HashResult; all: HashResult[] }) {
  const others = all.filter((o) => o !== r);
  const same = others.find((o) => o.sha256 === r.sha256);
  const differs = !same && others.find((o) => o.size === r.size && o.sha256 !== r.sha256);
  return (
    <>
      <div className="hash-head">
        <span className="hash-src" title={r.source}>
          <b>{baseName(r.source)}</b> <span className="dim mono">{r.source}</span>
        </span>
        <span className="dim small">
          {fmtBytes(r.size)} · {r.at.toLocaleTimeString()}
        </span>
      </div>
      <div className="hash-val">
        <code>{r.sha256}</code>
        <Button small onClick={() => void navigator.clipboard.writeText(r.sha256)}>
          Copy
        </Button>
      </div>
      {same && (
        <Badge tone="green">{same.source === r.source ? "Same as the earlier run" : `Matches ${baseName(same.source)}: identical`}</Badge>
      )}
      {differs && <Badge tone="red">Differs from {baseName(differs.source)}, though the size is the same</Badge>}
    </>
  );
}
