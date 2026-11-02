// Every tool, in menu order. Home's cards and the side menu both come from here.
import { FileSearch, Fingerprint, HardDrive, HardDriveDownload, Layers, type LucideIcon, Map as MapIcon, RotateCcw } from "lucide-react";
import type { Job } from "../lib/contract";
import type { Screen } from "../store/app";

export type Group = "Rescue" | "Inspect" | "Recover";

export interface Tool {
  id: string;
  screen: Screen;
  group: Group;
  label: string;
  icon: LucideIcon;
  sub: string;
  title: string;
  what: string;
  when: string;
  cta: string;
  /** The job whose progress this tool's card and menu entry show. */
  job?: Job;
}

export const TOOLS: Tool[] = [
  {
    id: "image",
    icon: HardDriveDownload,
    screen: "image",
    group: "Rescue",
    label: "Image",
    sub: "Copy a drive to a file",
    title: "Image a drive",
    what: "Copies the whole drive into an image file on a healthy drive. Readable areas go first; damaged areas are retried afterwards.",
    when: "The drive clicks, freezes or reports read errors.",
    cta: "Start imaging",
    job: "image",
  },
  {
    id: "retry",
    icon: RotateCcw,
    screen: "image",
    group: "Rescue",
    label: "Retry bad areas",
    sub: "Second pass only",
    title: "Retry bad areas",
    what: "Goes back over only the areas an earlier run could not read, sector by sector, using its map. Nothing already rescued is read again.",
    when: "An image finished with bad or pending areas left.",
    cta: "Retry bad areas",
    job: "image",
  },
  {
    id: "drives",
    icon: HardDrive,
    screen: "drives",
    group: "Inspect",
    label: "Drives",
    sub: "Details of each drive",
    title: "Drive details",
    what: "Every attached drive with its model, serial number, size, sector size and connection, and whether it is the system drive.",
    when: "You need to be sure which physical drive is which.",
    cta: "Show drives",
  },
  {
    id: "partitions",
    icon: Layers,
    screen: "partitions",
    group: "Inspect",
    label: "Partitions",
    sub: "Find lost volumes",
    title: "Find partitions",
    what: "Reads the partition table. A deep scan searches every sector for filesystems the table no longer lists.",
    when: "A volume vanished or shows as unformatted.",
    cta: "Scan for partitions",
    job: "scan",
  },
  {
    id: "mapview",
    icon: MapIcon,
    screen: "mapview",
    group: "Inspect",
    label: "Map viewer",
    sub: "Open a rescue map",
    title: "Map viewer",
    what: "Opens a map file from any earlier run and shows how much was rescued, what is bad, and where on the drive it is.",
    when: "You want to check an old rescue before resuming it.",
    cta: "Open a map",
  },
  {
    id: "verify",
    icon: Fingerprint,
    screen: "verify",
    group: "Inspect",
    label: "Verify",
    sub: "SHA-256 of a drive or image",
    title: "Verify an image",
    what: "Computes the SHA-256 of a whole image or drive. Two copies with the same hash are identical, byte for byte.",
    when: "You copied an image and want proof it is intact.",
    cta: "Compute a hash",
    job: "hash",
  },
  {
    id: "recover",
    icon: FileSearch,
    screen: "recover",
    group: "Recover",
    label: "Recover",
    sub: "Get files back",
    title: "Recover files",
    what: "Lists files on an NTFS volume, deleted ones too, and copies the ones you pick to a safe folder. Carving finds files by content when the filesystem is gone.",
    when: "Files were deleted, or the filesystem won't open.",
    cta: "Recover files",
  },
];

/** Each group's colour, used by its icons, labels and menu heading. */
export const GROUP_TONE: Record<Group, string> = { Rescue: "tone-amber", Inspect: "tone-accent", Recover: "tone-green" };
