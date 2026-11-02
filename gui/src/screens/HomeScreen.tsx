import { motion } from "motion/react";
import { DriveStrip } from "../components/DriveStrip";
import { StatusChip } from "../components/StatusChip";
import { GROUP_TONE, TOOLS, type Tool } from "../components/tools";
import { openTool, useToolStatus } from "../components/toolStatus";
import { cx } from "../components/ui";

const FACTS = [
  ["Read-only on the source", "Nothing is ever written to the drive you are rescuing."],
  ["Work on the copy", "Image a failing drive once, then recover from the image."],
  ["Stop any time", "Imaging keeps a map, so it resumes where it left off."],
  ["See what it's doing", "Ctrl+` opens the console with every command and its output."],
] as const;

const list = { show: { transition: { staggerChildren: 0.04 } } };
const item = { hidden: { opacity: 0, y: 12 }, show: { opacity: 1, y: 0, transition: { type: "spring", bounce: 0, visualDuration: 0.35 } } } as const;

export function HomeScreen() {
  return (
    <>
      <h2>Rescue data from a failing or damaged drive</h2>
      <p className="lead">
        Usually: image the drive, check its partitions, then recover files from the image. Every tool explains its settings on its own screen.
      </p>
      <DriveStrip />
      <h3 className="section-title">Tools</h3>
      <motion.div className="tools" variants={list} initial="hidden" animate="show">
        {TOOLS.map((t) => (
          <ToolCard key={t.id} tool={t} />
        ))}
        <motion.section className="tool facts" variants={item} aria-label="Good to know">
          <h3>Good to know</h3>
          {FACTS.map(([k, v]) => (
            <p key={k}>
              <b>{k}.</b> <span className="dim">{v}</span>
            </p>
          ))}
        </motion.section>
      </motion.div>
    </>
  );
}

function ToolCard({ tool }: { tool: Tool }) {
  const status = useToolStatus(tool);
  return (
    <motion.button
      type="button"
      className={cx("tool", status.busy && "busy-card")}
      variants={item}
      whileHover={{ y: -3 }}
      whileTap={{ scale: 0.985 }}
      transition={{ type: "spring", bounce: 0, visualDuration: 0.2 }}
      onClick={() => openTool(tool)}
      aria-describedby={`tool-${tool.id}-what`}
    >
      <span className="tool-top">
        <span className={cx("tool-group", GROUP_TONE[tool.group])}>{tool.group}</span>
        <StatusChip status={status} />
      </span>
      <span className="tool-title">
        <span className={cx("tool-icon", GROUP_TONE[tool.group])}>
          <tool.icon size={18} strokeWidth={2} aria-hidden="true" />
        </span>
        {tool.title}
      </span>
      <span className="tool-what" id={`tool-${tool.id}-what`}>
        {tool.what}
      </span>
      <span className="tool-when">
        <b>Use it when:</b> {tool.when}
      </span>
      <span className="tool-cta">{tool.cta} →</span>
    </motion.button>
  );
}
