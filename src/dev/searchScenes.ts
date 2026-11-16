import { people } from "../data/demo";
import { noFilters } from "../lib/search";
import { demoAi, setAi } from "../services/ai";
import { useAiSettings } from "../state/aiSettings";
import { useSearch } from "../state/search";
import { useStore } from "../state/store";
import type { Scene } from "./scenes";

/** The AI as a scene draws it: the demo at `start`, its status on screen at once. */
export function aiScene(start: Parameters<typeof demoAi>[0]) {
  const demo = demoAi(start);
  setAi(demo);
  void demo.status().then((status) => useAiSettings.setState({ status }));
}

export const searchScenes: Record<string, Scene> = {
  "search-results": {
    shows: /Best matches/,
    apply: () => {
      useStore.getState().openSearch();
      useSearch.setState({ query: "budget", filters: { ...noFilters, from: people.daniel } });
    },
  },
  "search-without-ai": {
    shows: /Search by meaning needs AI/,
    apply: () => {
      aiScene({ plan: "skip" });
      useStore.getState().openSearch();
      useSearch.setState({ query: "budget", filters: { ...noFilters, from: people.daniel } });
    },
  },
};
