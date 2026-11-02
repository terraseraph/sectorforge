import { useApp } from "../store/app";
import { useImage } from "../store/image";
import { files, selection, useRecover } from "../store/recover";
import { useScan } from "../store/scan";

const initial = {
  app: useApp.getState(),
  image: useImage.getState(),
  scan: useScan.getState(),
  recover: useRecover.getState(),
};

/** Puts every store back to its startup state. */
export function resetStores() {
  useApp.setState(initial.app, true);
  useImage.setState(initial.image, true);
  useScan.setState(initial.scan, true);
  useRecover.setState(initial.recover, true);
  files.clear();
  selection.clear();
}
