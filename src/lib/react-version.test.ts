import { describe, expect, it } from "vitest";
import packageJson from "../../package.json";

/**
 * React and ReactDOM must be the EXACT SAME version.
 *
 * They used to be declared as `^19.3.0` and `^19.3.1`, which resolved
 * to react@19.3.0 and react-dom@19.3.1 — and react-dom declares
 * `peerDependencies: { react: "^19.3.1" }`, so the install violated its
 * own peer requirement.
 *
 * React 19 then throws at runtime:
 *
 *   Incompatible React versions: The "react" and "react-dom" packages
 *   must have the exact same version.
 *
 * The symptom is a BLANK WINDOW with no error the user can see: the
 * throw happens inside react-dom's client entry point, before the app
 * mounts, so nothing renders at all. Every screenshot of the app taken
 * during the 1.0.0 launch preparation was an empty white rectangle.
 *
 * The caret ranges are the trap. `^19.3.0` is satisfied by 19.3.0, and
 * `^19.3.1` is satisfied by 19.3.1, and a resolver is free to pick
 * different points in the two ranges. Pinning both to the same exact
 * version removes that freedom.
 *
 * This test reads `package.json` through Vite's JSON import rather than
 * the filesystem, so it needs no Node types — the project's `tsconfig`
 * deliberately allows only `vite/client` types, and adding
 * `@types/node` for one test would widen that for the whole app.
 */
const pkg = packageJson as unknown as {
  dependencies: Record<string, string>;
  devDependencies: Record<string, string>;
};

describe("react version pinning", () => {
  it("declares react and react-dom at the same exact version", () => {
    const react = pkg.dependencies.react;
    const reactDom = pkg.dependencies["react-dom"];

    expect(react, "react must be a dependency").toBeDefined();
    expect(reactDom, "react-dom must be a dependency").toBeDefined();
    expect(react, "react and react-dom must declare the same version").toBe(
      reactDom,
    );

    // An exact pin, not a range. A caret range lets the resolver pick
    // two different points in the two ranges, which is what caused this.
    expect(
      react,
      "react must be pinned exactly — a range lets react and react-dom drift apart",
    ).toMatch(/^\d+\.\d+\.\d+$/);
    expect(
      reactDom,
      "react-dom must be pinned exactly — a range lets react and react-dom drift apart",
    ).toMatch(/^\d+\.\d+\.\d+$/);
  });

  it("pins the type packages to the same version", () => {
    const types = pkg.devDependencies["@types/react"];
    const typesDom = pkg.devDependencies["@types/react-dom"];

    expect(types, "@types/react must be a devDependency").toBeDefined();
    expect(typesDom, "@types/react-dom must be a devDependency").toBeDefined();
    expect(types, "the React type packages must declare the same version").toBe(
      typesDom,
    );
  });

  it("satisfies react-dom's own peer requirement", () => {
    // react-dom@19.3.1 declares `peerDependencies: { react: "^19.3.1" }`.
    // If react is pinned BELOW that, the install violates the peer
    // requirement and React throws at runtime. This is the assertion
    // that would have caught the original bug.
    const react = pkg.dependencies.react;
    const reactDom = pkg.dependencies["react-dom"];

    const [reactMajor, reactMinor] = react.split(".").map(Number);
    const [domMajor, domMinor] = reactDom.split(".").map(Number);

    expect(
      reactMajor === domMajor && reactMinor >= domMinor,
      `react@${react} does not satisfy react-dom@${reactDom}'s peer requirement of react ^${domMajor}.${domMinor}`,
    ).toBe(true);
  });
});
