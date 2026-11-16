import { providers } from "../../data/providers";
import { ScreenHeading } from "../../design-system/molecules";
import { ProviderList } from "../../design-system/organisms";
import { FlowLayout } from "../../design-system/templates";
import { useAddAccount } from "../../state/addAccount";
import { Footnote } from "./parts";

export function ChooseProvider() {
  const cursor = useAddAccount((s) => s.cursor);
  const pick = useAddAccount((s) => s.pick);
  return (
    <FlowLayout gap={28}>
      <ScreenHeading title="Which email do you want to add?" sub="Pick your provider. Moss then shows the easiest way to sign in." />
      <ProviderList providers={providers} cursor={cursor} onPick={pick} />
      <Footnote>Moss keeps your passwords and keys in the macOS Keychain, never in a plain file.</Footnote>
    </FlowLayout>
  );
}
