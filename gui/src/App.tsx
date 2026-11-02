import { AnimatePresence, MotionConfig, motion } from "motion/react";
import { Console } from "./components/Console";
import { CliBanner, Nav, StatusBar } from "./components/Shell";
import { DrivesScreen } from "./screens/DrivesScreen";
import { HomeScreen } from "./screens/HomeScreen";
import { ImageScreen } from "./screens/ImageScreen";
import { MapViewScreen } from "./screens/MapViewScreen";
import { PartitionsScreen } from "./screens/PartitionsScreen";
import { RecoverScreen } from "./screens/recover/RecoverScreen";
import { VerifyScreen } from "./screens/VerifyScreen";
import { type Screen, useApp } from "./store/app";

const SCREENS: Record<Screen, () => React.JSX.Element> = {
  home: HomeScreen,
  image: ImageScreen,
  partitions: PartitionsScreen,
  recover: RecoverScreen,
  drives: DrivesScreen,
  mapview: MapViewScreen,
  verify: VerifyScreen,
};

export function App() {
  const screen = useApp((s) => s.screen);
  const Current = SCREENS[screen];
  return (
    // Honour the OS "reduce motion" setting everywhere.
    <MotionConfig reducedMotion="user">
      <div className="shell">
        <Nav />
        <div className="content">
          <CliBanner />
          <main>
            <AnimatePresence mode="wait" initial={false}>
              <motion.div
                key={screen}
                initial={{ opacity: 0, y: 6 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0, y: -4 }}
                transition={{ duration: 0.16, ease: "easeOut" }}
              >
                <Current />
              </motion.div>
            </AnimatePresence>
          </main>
          <Console />
        </div>
        <StatusBar />
      </div>
    </MotionConfig>
  );
}
