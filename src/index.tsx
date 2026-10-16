import { ChakraProvider, defaultSystem } from "@chakra-ui/react";
import { loader } from "@monaco-editor/react";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import * as wasm from "rustpad-wasm";

import App from "./App";
import "./index.css";
import { Toaster } from "./toaster";

loader.config({
  paths: {
    vs: `https://cdn.jsdelivr.net/npm/monaco-editor@${__MONACO_VERSION__}/min/vs`,
  },
});

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <ChakraProvider value={defaultSystem}>
      <App />
      <Toaster />
    </ChakraProvider>
  </StrictMode>,
);
