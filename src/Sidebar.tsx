import {
  Button,
  Container,
  Flex,
  Heading,
  Input,
  InputGroup,
  Link,
  NativeSelect,
  Stack,
  Switch,
  Text,
} from "@chakra-ui/react";
import { VscRepo } from "react-icons/vsc";

import ConnectionStatus from "./ConnectionStatus";
import User from "./User";
import languages from "./languages.json";
import type { UserInfo } from "./rustpad";
import { toaster } from "./toaster";

export type SidebarProps = {
  documentId: string;
  connection: "connected" | "disconnected" | "desynchronized";
  darkMode: boolean;
  language: string;
  currentUser: UserInfo;
  users: Record<number, UserInfo>;
  onDarkModeChange: () => void;
  onLanguageChange: (language: string) => void;
  onLoadSample: () => void;
  onChangeName: (name: string) => void;
  onChangeColor: () => void;
};

function Sidebar({
  documentId,
  connection,
  darkMode,
  language,
  currentUser,
  users,
  onDarkModeChange,
  onLanguageChange,
  onLoadSample,
  onChangeName,
  onChangeColor,
}: SidebarProps) {
  // For sharing the document by link to others.
  const documentUrl = `${window.location.origin}/#${documentId}`;

  async function handleCopy() {
    await navigator.clipboard.writeText(documentUrl);
    toaster.create({
      title: "Copied!",
      description: "Link copied to clipboard",
      type: "success",
      duration: 2000,
      closable: true,
    });
  }

  return (
    <Container
      w={{ base: "3xs", md: "2xs", lg: "xs" }}
      display={{ base: "none", sm: "block" }}
      bgColor={darkMode ? "#252526" : "#f3f3f3"}
      overflowY="auto"
      maxW="full"
      lineHeight={1.4}
      py={4}
    >
      <ConnectionStatus darkMode={darkMode} connection={connection} />

      <Flex justifyContent="space-between" mt={4} mb={1.5} w="full">
        <Heading size="sm">Dark Mode</Heading>
        <Switch.Root
          size="sm"
          checked={darkMode}
          onCheckedChange={onDarkModeChange}
        >
          <Switch.HiddenInput aria-label="Dark mode" />
          <Switch.Control>
            <Switch.Thumb />
          </Switch.Control>
        </Switch.Root>
      </Flex>

      <Heading mt={4} mb={1.5} size="sm">
        Language
      </Heading>
      <NativeSelect.Root size="sm">
        <NativeSelect.Field
          aria-label="Language"
          bgColor={darkMode ? "#3c3c3c" : "white"}
          borderColor={darkMode ? "#3c3c3c" : "white"}
          value={language}
          onChange={(event) => onLanguageChange(event.target.value)}
        >
          {languages.map((lang) => (
            <option key={lang} value={lang} style={{ color: "black" }}>
              {lang}
            </option>
          ))}
        </NativeSelect.Field>
        <NativeSelect.Indicator />
      </NativeSelect.Root>

      <Heading mt={4} mb={1.5} size="sm">
        Share Link
      </Heading>
      <InputGroup
        endElementProps={{ pe: 1 }}
        endElement={
          <Button
            h="1.4rem"
            size="xs"
            onClick={handleCopy}
            _hover={{ bg: darkMode ? "#575759" : "gray.200" }}
            bgColor={darkMode ? "#575759" : "gray.200"}
            color={darkMode ? "white" : "inherit"}
          >
            Copy
          </Button>
        }
      >
        <Input
          readOnly
          size="sm"
          variant="outline"
          bgColor={darkMode ? "#3c3c3c" : "white"}
          borderColor={darkMode ? "#3c3c3c" : "white"}
          value={documentUrl}
        />
      </InputGroup>

      <Heading mt={4} mb={1.5} size="sm">
        Active Users
      </Heading>
      <Stack gap={0} mb={1.5} fontSize="sm">
        <User
          info={currentUser}
          isMe
          onChangeName={onChangeName}
          onChangeColor={onChangeColor}
          darkMode={darkMode}
        />
        {Object.entries(users).map(([id, info]) => (
          <User key={id} info={info} darkMode={darkMode} />
        ))}
      </Stack>

      <Heading mt={4} mb={1.5} size="sm">
        About
      </Heading>
      <Text fontSize="sm" mb={1.5}>
        <strong>Rustpad</strong> is an open-source collaborative text editor
        based on the <em>operational transformation</em> algorithm.
      </Text>
      <Text fontSize="sm" mb={1.5}>
        Share a link to this pad with others, and they can edit from their
        browser while seeing your changes in real time.
      </Text>
      <Text fontSize="sm" mb={1.5}>
        Built using Rust and TypeScript. See the{" "}
        <Link
          color="blue.600"
          fontWeight="semibold"
          href="https://github.com/ekzhang/rustpad"
          target="_blank"
          rel="noopener noreferrer"
        >
          GitHub repository
        </Link>{" "}
        for details.
      </Text>

      <Button
        size="sm"
        borderColor={darkMode ? "purple.400" : "purple.600"}
        color={darkMode ? "purple.400" : "purple.600"}
        _hover={{ bg: darkMode ? "whiteAlpha.200" : "blackAlpha.100" }}
        variant="outline"
        mt={1}
        onClick={onLoadSample}
      >
        <VscRepo />
        Read the code
      </Button>
    </Container>
  );
}

export default Sidebar;
