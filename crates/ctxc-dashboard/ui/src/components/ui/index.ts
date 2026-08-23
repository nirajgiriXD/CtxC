/**
 * The shadcn/ui layer.
 *
 * shadcn/ui is copied into a project rather than installed, so these are the
 * components themselves — same tokens, same `cn()` helper, same variant shapes
 * as upstream. Only what the dashboard actually renders is here; there is no
 * value in carrying components nothing uses.
 *
 * One entry point, so a page imports from `components/ui` rather than from
 * eleven files.
 */

export { Button, buttonVariants } from "./button";
export { Badge, Dot } from "./badge";
export {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardHeading,
  CardTitle,
} from "./card";
export {
  Command,
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
  CommandShortcut,
} from "./command";
export {
  Confirm,
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "./dialog";
export {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuShortcut,
  DropdownMenuTrigger,
} from "./dropdown-menu";
export { Field, FieldRow, Input, Label, Textarea } from "./field";
export { Kbd, Meter, ScrollArea, Separator } from "./misc";
export {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectSeparator,
  SelectTrigger,
  SelectValue,
} from "./select";
export { Code, Empty, Failure, Notice, Skeleton, SkeletonRows } from "./states";
export { Switch, SwitchField } from "./switch";
export { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "./table";
export { Tabs, TabsContent, TabsList, TabsTrigger } from "./tabs";
export { Toaster, toast } from "./toast";
export { Tooltip, TooltipContent, TooltipProvider, TooltipRoot, TooltipTrigger } from "./tooltip";
