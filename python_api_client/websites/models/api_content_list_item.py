from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, cast
from uuid import UUID

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

T = TypeVar("T", bound="ApiContentListItem")


@_attrs_define
class ApiContentListItem:
    created_at: str
    creator_sub: str
    draft: bool
    id: UUID
    page_type: str
    site_id: UUID
    slug: str
    tags: list[str]
    title: str
    last_updated: None | str | Unset = UNSET
    published_at: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)

    def to_dict(self) -> dict[str, Any]:
        created_at = self.created_at

        creator_sub = self.creator_sub

        draft = self.draft

        id = str(self.id)

        page_type = self.page_type

        site_id = str(self.site_id)

        slug = self.slug

        tags = self.tags

        title = self.title

        last_updated: None | str | Unset
        if isinstance(self.last_updated, Unset):
            last_updated = UNSET
        else:
            last_updated = self.last_updated

        published_at: None | str | Unset
        if isinstance(self.published_at, Unset):
            published_at = UNSET
        else:
            published_at = self.published_at

        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update(
            {
                "created_at": created_at,
                "creator_sub": creator_sub,
                "draft": draft,
                "id": id,
                "page_type": page_type,
                "site_id": site_id,
                "slug": slug,
                "tags": tags,
                "title": title,
            }
        )
        if last_updated is not UNSET:
            field_dict["last_updated"] = last_updated
        if published_at is not UNSET:
            field_dict["published_at"] = published_at

        return field_dict

    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        created_at = d.pop("created_at")

        creator_sub = d.pop("creator_sub")

        draft = d.pop("draft")

        id = UUID(d.pop("id"))

        page_type = d.pop("page_type")

        site_id = UUID(d.pop("site_id"))

        slug = d.pop("slug")

        tags = cast(list[str], d.pop("tags"))

        title = d.pop("title")

        def _parse_last_updated(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        last_updated = _parse_last_updated(d.pop("last_updated", UNSET))

        def _parse_published_at(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        published_at = _parse_published_at(d.pop("published_at", UNSET))

        api_content_list_item = cls(
            created_at=created_at,
            creator_sub=creator_sub,
            draft=draft,
            id=id,
            page_type=page_type,
            site_id=site_id,
            slug=slug,
            tags=tags,
            title=title,
            last_updated=last_updated,
            published_at=published_at,
        )

        api_content_list_item.additional_properties = d
        return api_content_list_item

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
