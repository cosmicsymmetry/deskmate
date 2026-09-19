import { GlobalRegistrator } from "@happy-dom/global-registrator";

// happy-dom supplies window/document for all live-mounted frontend suites.
// Static markup tests share that environment but do not run component effects.
GlobalRegistrator.register();

// Tells React it is safe to batch/flush effects synchronously inside `act()` in this
// environment — without it, React logs "not configured to support act(...)" on every
// `act()` call even though the calls themselves behave correctly.
declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}
globalThis.IS_REACT_ACT_ENVIRONMENT = true;
