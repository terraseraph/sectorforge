import { Button } from "../components/ui";
import { goTo } from "../store/actions";
import type { Screen } from "../store/app";

const TOOLS: {
  id: Screen;
  step: number;
  title: string;
  cta: string;
  what: string;
  when: string[];
  out: string;
}[] = [
  {
    id: "image",
    step: 1,
    title: "Image a drive",
    cta: "Start imaging",
    what: "Copies the whole drive, sector by sector, into an image file on a healthy drive. Readable areas are copied first; damaged areas are retried afterwards.",
    when: ["The drive clicks, freezes, or reports read errors", "You want one careful read of the drive, then never touch it again"],
    out: "An image file plus a map of what was rescued. Pause any time; it resumes where it stopped.",
  },
  {
    id: "partitions",
    step: 2,
    title: "Find partitions",
    cta: "Scan for partitions",
    what: "Reads the partition table to show the volumes on a drive or image. A deep scan searches every sector for filesystems the table no longer lists.",
    when: ["A volume has vanished or shows as unformatted", "You need to know which partition to recover from"],
    out: "A list of partitions with their filesystem, position, size, and how sure SectorForge is.",
  },
  {
    id: "recover",
    step: 3,
    title: "Recover files",
    cta: "Recover files",
    what: "Lists files on an NTFS, FAT, exFAT or ext volume, including deleted ones, and copies the ones you pick to a safe folder. Carving finds files by their contents when the filesystem is gone.",
    when: ["You deleted files by mistake", "The filesystem is too damaged to open"],
    out: "Your files, saved to a folder you choose. Files over bad sectors are flagged as partial.",
  },
];

const FACTS = [
  ["Read-only on the source", "Nothing here writes to the drive you are rescuing. Output always goes somewhere else."],
  ["Work on the copy", "Every read stresses a failing drive. Image it once, then recover from the image as often as you like."],
  ["Stop any time", "Every job can be stopped. Imaging keeps a map, so it carries on later without starting over."],
] as const;

export function HomeScreen() {
  return (
    <>
      <h2>Rescue data from a failing or damaged drive</h2>
      <p className="lead">Three tools, usually used in this order. Pick one to start; each screen explains its own settings.</p>
      <div className="tools">
        {TOOLS.map((t) => (
          <section key={t.id} className="tool" aria-labelledby={`tool-${t.id}`}>
            <div className="tool-step">Step {t.step}</div>
            <h3 id={`tool-${t.id}`}>{t.title}</h3>
            <p>{t.what}</p>
            <h4>Use it when</h4>
            <ul>
              {t.when.map((w) => (
                <li key={w}>{w}</li>
              ))}
            </ul>
            <h4>You get</h4>
            <p>{t.out}</p>
            <Button variant="primary" onClick={() => goTo(t.id)}>
              {t.cta}
            </Button>
          </section>
        ))}
      </div>
      <div className="facts">
        {FACTS.map(([k, v]) => (
          <div key={k}>
            <b>{k}.</b> <span className="dim">{v}</span>
          </div>
        ))}
      </div>
    </>
  );
}
