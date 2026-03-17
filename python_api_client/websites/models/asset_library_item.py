from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, cast
from uuid import UUID

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

T = TypeVar("T", bound="AssetLibraryItem")


@_attrs_define
class AssetLibraryItem:
    created_at: str
    has_thumbnail: bool
    id: UUID
    mime_type: str
    original_filename: str
    original_url: str
    height: int | None | Unset = UNSET
    thumbnail_url: None | str | Unset = UNSET
    width: int | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)

    def to_dict(self) -> dict[str, Any]:
        created_at = self.created_at

        has_thumbnail = self.has_thumbnail

        id = str(self.id)

        mime_type = self.mime_type

        original_filename = self.original_filename

        original_url = self.original_url

        height: int | None | Unset
        if isinstance(self.height, Unset):
            height = UNSET
        else:
            height = self.height

        thumbnail_url: None | str | Unset
        if isinstance(self.thumbnail_url, Unset):
            thumbnail_url = UNSET
        else:
            thumbnail_url = self.thumbnail_url

        width: int | None | Unset
        if isinstance(self.width, Unset):
            width = UNSET
        else:
            width = self.width

        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update(
            {
                "created_at": created_at,
                "has_thumbnail": has_thumbnail,
                "id": id,
                "mime_type": mime_type,
                "original_filename": original_filename,
                "original_url": original_url,
            }
        )
        if height is not UNSET:
            field_dict["height"] = height
        if thumbnail_url is not UNSET:
            field_dict["thumbnail_url"] = thumbnail_url
        if width is not UNSET:
            field_dict["width"] = width

        return field_dict

    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        created_at = d.pop("created_at")

        has_thumbnail = d.pop("has_thumbnail")

        id = UUID(d.pop("id"))

        mime_type = d.pop("mime_type")

        original_filename = d.pop("original_filename")

        original_url = d.pop("original_url")

        def _parse_height(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        height = _parse_height(d.pop("height", UNSET))

        def _parse_thumbnail_url(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        thumbnail_url = _parse_thumbnail_url(d.pop("thumbnail_url", UNSET))

        def _parse_width(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        width = _parse_width(d.pop("width", UNSET))

        asset_library_item = cls(
            created_at=created_at,
            has_thumbnail=has_thumbnail,
            id=id,
            mime_type=mime_type,
            original_filename=original_filename,
            original_url=original_url,
            height=height,
            thumbnail_url=thumbnail_url,
            width=width,
        )

        asset_library_item.additional_properties = d
        return asset_library_item

    @property
    def additional_keys(self) -> list[str]:
        return list(self.additional_properties.keys())

    def __getitem__(self, key: str) -> Any:
        return self.additional_properties[key]

    def __setitem__(self, key: str, value: Any) -> None:
        self.additional_properties[key] = value

    def __delitem__(self, key: str) -> None:
        del self.additional_properties[key]

    def __contains__(self, key: str) -> bool:
        return key in self.additional_properties
