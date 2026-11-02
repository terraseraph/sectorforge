import { create } from "zustand";

/** save: a folder plus a file name; folder: a folder; open: an existing file. */
export type PickMode = "save" | "folder" | "open";

export interface PickRequest {
  /** New for every request, so the picker starts fresh. */
  id: number;
  title: string;
  mode: PickMode;
  /** Pre-filled file name (save), or a full path whose folder to start in. */
  initial?: string;
  /** Bytes the file will need; drives with less free space are flagged. */
  need?: number;
  /** The full path chosen, or null on Cancel. */
  resolve: (path: string | null) => void;
}

export const usePicker = create<{ req: PickRequest | null; lastDir: string | null }>(() => ({ req: null, lastDir: null }));

let asked = 0;

/** The in-app file and folder picker, for when the native one isn't available (a browser tab). */
export function pickPath(opts: Omit<PickRequest, "id" | "resolve">): Promise<string | null> {
  usePicker.getState().req?.resolve(null);
  return new Promise((resolve) =>
    usePicker.setState({
      req: {
        ...opts,
        id: ++asked,
        resolve: (path) => {
          usePicker.setState({ req: null });
          resolve(path);
        },
      },
    }),
  );
}
