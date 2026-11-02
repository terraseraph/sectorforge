import { CliBanner, Nav, StatusBar } from "./components/Shell";
import { ImageScreen } from "./screens/ImageScreen";
import { PartitionsScreen } from "./screens/PartitionsScreen";
import { RecoverScreen } from "./screens/recover/RecoverScreen";
import { useApp } from "./store/app";

export function App() {
  const screen = useApp((s) => s.screen);
  return (
    <div className="shell">
      <Nav />
      <div className="content">
        <CliBanner />
        <main>
          {screen === "image" && <ImageScreen />}
          {screen === "partitions" && <PartitionsScreen />}
          {screen === "recover" && <RecoverScreen />}
        </main>
      </div>
      <StatusBar />
    </div>
  );
}
