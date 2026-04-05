from __future__ import annotations

from collections.abc import Mapping
from typing import TYPE_CHECKING, Any, TypeVar

from attrs import define as _attrs_define
from attrs import field as _attrs_field

if TYPE_CHECKING:
    from ..models.api_asset_detail import ApiAssetDetail


T = TypeVar("T", bound="ApiAssetBatchResponse")


@_attrs_define
class ApiAssetBatchResponse:
    assets: list[ApiAssetDetail]
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)

    def to_dict(self) -> dict[str, Any]:
        assets = []
        for assets_item_data in self.assets:
            assets_item = assets_item_data.to_dict()
            assets.append(assets_item)

        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update(
            {
                "assets": assets,
            }
        )

        return field_dict

    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.api_asset_detail import ApiAssetDetail

        d = dict(src_dict)
        assets = []
        _assets = d.pop("assets")
        for assets_item_data in _assets:
            assets_item = ApiAssetDetail.from_dict(assets_item_data)

            assets.append(assets_item)

        api_asset_batch_response = cls(
            assets=assets,
        )

        api_asset_batch_response.additional_properties = d
        return api_asset_batch_response

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
