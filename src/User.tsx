import {
  Button,
  CloseButton,
  HStack,
  Icon,
  Input,
  Popover,
  Portal,
  Text,
} from "@chakra-ui/react";
import { useRef, useState } from "react";
import { FaPalette } from "react-icons/fa";
import { VscAccount } from "react-icons/vsc";

import { UserInfo } from "./rustpad";

type UserProps = {
  info: UserInfo;
  isMe?: boolean;
  onChangeName?: (name: string) => void;
  onChangeColor?: () => void;
  darkMode: boolean;
};

function User({
  info,
  isMe = false,
  onChangeName,
  onChangeColor,
  darkMode,
}: UserProps) {
  const inputRef = useRef<HTMLInputElement>(null);
  const [isOpen, setOpen] = useState(false);

  const nameColor = `hsl(${info.hue}, 90%, ${darkMode ? "70%" : "25%"})`;
  const borderColor = darkMode ? "#464647" : "gray.200";
  return (
    <Popover.Root
      positioning={{ placement: "right" }}
      open={isOpen}
      onOpenChange={(details) => setOpen(isMe && details.open)}
      initialFocusEl={() => inputRef.current}
    >
      <Popover.Trigger asChild>
        <HStack
          p={2}
          rounded="md"
          _hover={{
            bgColor: darkMode ? "#464647" : "gray.200",
            cursor: "pointer",
          }}
        >
          <Icon as={VscAccount} />
          <Text fontWeight="medium" color={nameColor}>
            {info.name}
          </Text>
          {isMe && <Text>(you)</Text>}
        </HStack>
      </Popover.Trigger>
      <Portal>
        <Popover.Positioner>
          <Popover.Content
            bgColor={darkMode ? "#333333" : "white"}
            color={darkMode ? "#cbcaca" : "inherit"}
            borderColor={borderColor}
          >
            <Popover.Arrow>
              <Popover.ArrowTip bgColor={darkMode ? "#333333" : "white"} />
            </Popover.Arrow>
            <Popover.Header
              fontWeight="semibold"
              borderBottomWidth="1px"
              borderColor={borderColor}
            >
              <Popover.Title>Update Info</Popover.Title>
            </Popover.Header>
            <Popover.CloseTrigger asChild position="absolute" top={1} right={1}>
              <CloseButton size="sm" />
            </Popover.CloseTrigger>
            <Popover.Body>
              <Input
                ref={inputRef}
                mb={2}
                value={info.name}
                maxLength={25}
                onChange={(event) => onChangeName?.(event.target.value)}
              />
              <Button
                size="sm"
                w="100%"
                variant="subtle"
                colorPalette="gray"
                onClick={onChangeColor}
              >
                <FaPalette />
                Change Color
              </Button>
            </Popover.Body>
            <Popover.Footer
              display="flex"
              justifyContent="flex-end"
              borderTopWidth="1px"
              borderColor={borderColor}
            >
              <Button
                size="sm"
                colorPalette="blue"
                onClick={() => setOpen(false)}
              >
                Done
              </Button>
            </Popover.Footer>
          </Popover.Content>
        </Popover.Positioner>
      </Portal>
    </Popover.Root>
  );
}

export default User;
